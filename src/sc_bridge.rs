//! SystemC co-simulation bridge.
//!
//! Runtime-optional: without any `--sc-*` flag nothing in this module runs and
//! the simulator behaves exactly as before (no build-time dependency on
//! SystemC; the C++ side is compiled and `dlopen`-ed lazily).
//!
//! Two integration modes, selected by `--sc-top`:
//!
//! * **SV top** (`sv`, default): xezim is the time master. The user's SystemC
//!   model sources are compiled together with `src/sc_shim/sc_shim.cc` into a
//!   model shared object exposing `xsc_model_init()`. The SV testbench drives
//!   it explicitly with `$sc_step`, `$sc_export_sig`, `$sc_import_sig` and
//!   `$sc_finish`.
//!
//! * **SC top** (`sc`): SystemC is the time master. The user's `sc_main()`
//!   sources are compiled together with the shim into a cosim shared object
//!   exposing `xsc_cosim_main()`; xezim runs its kernel on a worker thread
//!   that rendezvouses with the spawned `xezim_host` SystemC thread once per
//!   slice (bounded by `--sc-quantum`). SystemVerilog signals are attached
//!   with `--sc-map sv_name=sc_name:dir`.
//!
//! Time mapping: the SystemC time resolution is set to exactly one xezim
//! tick, so one `sc_time` value unit == one xezim tick everywhere at the
//! boundary.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

use xezim_core::Value;

// ── CLI-facing configuration ────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScTop {
    Sv,
    Sc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScDir {
    /// SystemVerilog drives the SystemC signal ($sc_export_sig / `:out`).
    Export,
    /// SystemC drives the SystemVerilog signal ($sc_import_sig / `:in`).
    Import,
}

/// Everything the CLI collects; consumed once at simulation start.
#[derive(Clone, Debug, Default)]
pub struct ScOptions {
    pub top: Option<ScTop>,
    /// User C++ sources (model for SV top; sc_main TU for SC top).
    pub sources: Vec<String>,
    pub include_dirs: Vec<String>,
    pub cxxflags: Vec<String>,
    /// Prebuilt model .so (built from the user TU + our sc_shim.cc); skips
    /// the compile step.
    pub libs: Vec<String>,
    /// (sv_hier_name, sc_name, dir) — SC top; resolved after elaboration.
    pub maps: Vec<(String, String, ScDir)>,
    /// Max ticks the SystemC kernel may run ahead per slice (SC top).
    pub quantum: u64,
}

static SC_OPTIONS: OnceLock<Mutex<Option<ScOptions>>> = OnceLock::new();

pub fn set_sc_options(opts: ScOptions) {
    let cell = SC_OPTIONS.get_or_init(|| Mutex::new(None));
    *cell.lock().unwrap() = Some(opts);
}

pub fn sc_options() -> Option<ScOptions> {
    let cell = SC_OPTIONS.get_or_init(|| Mutex::new(None));
    cell.lock().unwrap().clone()
}

pub fn sc_top() -> ScTop {
    sc_options().and_then(|o| o.top).unwrap_or(ScTop::Sv)
}

// Paths of the shim sources, baked by build.rs from the crate manifest dir.
pub fn shim_source() -> &'static str {
    option_env!("XEZIM_SC_SHIM_SRC").unwrap_or("src/sc_shim/sc_shim.cc")
}

pub fn shim_include_dir() -> &'static str {
    option_env!("XEZIM_SC_SHIM_INC").unwrap_or("src/sc_shim")
}

// ── Shared link between the two kernels (SC top) ────────────────────────────

const CAP_INITIAL: u64 = u64::MAX;

/// State shared between the xezim worker thread and the SystemC host thread.
/// The blocking rendezvous itself lives in the shim (`xsc_sv_wait_slice`);
/// this struct carries control flags and the loaded shim handle.
pub struct ScLink {
    /// SystemC master's time as of the last completed slice (ticks).
    cap: AtomicU64,
    /// Set when the SystemC side is done (sc_main returned / failed).
    sc_done: AtomicBool,
    /// Set by the SV worker as soon as the elaborated tick length is known.
    tick_s_bits: AtomicU64,
    tick_mu: Mutex<()>,
    tick_ready: AtomicBool,
    tick_cv: Condvar,
    /// The cosim shared object's API, published by the SystemC host thread
    /// once the library is loaded and `xsc_cosim_main` entered.
    shim: Mutex<Option<Arc<ScShimApi>>>,
    /// Set when the SV kernel has finished (used for diagnostics only; the
    /// shim's own g_sv_finished flag carries the signal into SystemC).
    sv_finished: AtomicBool,
}

impl ScLink {
    pub fn new() -> Self {
        Self {
            cap: AtomicU64::new(CAP_INITIAL),
            sc_done: AtomicBool::new(false),
            tick_s_bits: AtomicU64::new(0),
            tick_mu: Mutex::new(()),
            tick_ready: AtomicBool::new(false),
            tick_cv: Condvar::new(),
            shim: Mutex::new(None),
            sv_finished: AtomicBool::new(false),
        }
    }

    pub fn current_cap(&self) -> u64 {
        self.cap.load(Ordering::Acquire)
    }

    pub fn set_cap(&self, cap: u64) {
        self.cap.store(cap, Ordering::Release);
    }

    pub fn sc_done(&self) -> bool {
        self.sc_done.load(Ordering::Acquire)
    }

    pub fn mark_sc_done(&self) {
        self.sc_done.store(true, Ordering::Release);
    }

    /// Called by the xezim worker thread once `tick_s` is known.
    pub fn set_tick_s(&self, tick_s: f64) {
        self.tick_s_bits
            .store(tick_s.to_bits(), Ordering::Release);
        self.tick_ready.store(true, Ordering::Release);
        self.tick_cv.notify_all();
    }

    /// Called by the SystemC host thread; waits for the tick length. Returns
    /// None if the SV side finished first (never compiled / fatal error).
    pub fn wait_tick_s(&self) -> Option<f64> {
        let mu = self.tick_mu.lock().unwrap();
        let mut guard: MutexGuard<'_, ()> = mu;
        while !self.tick_ready.load(Ordering::Acquire) && !self.sc_done() {
            // Poll with a timeout so a finished-but-unwoken host thread still
            // observes sc_done.
            let (g, _) = self
                .tick_cv
                .wait_timeout(guard, std::time::Duration::from_millis(100))
                .unwrap();
            guard = g;
        }
        if !self.tick_ready.load(Ordering::Acquire) {
            return None;
        }
        Some(f64::from_bits(self.tick_s_bits.load(Ordering::Acquire)))
    }

    pub fn publish_shim(&self, shim: Arc<ScShimApi>) {
        *self.shim.lock().unwrap() = Some(shim);
    }

    /// Wait for the SystemC host thread to publish its model handle. Returns
    /// None when the host gave up (its error is already on stderr) — callers
    /// then run the SystemVerilog side standalone. Blocks: building the model
    /// (g++ + SystemC headers) takes seconds, and the SV worker gets here
    /// first.
    pub fn take_shim(&self) -> Option<Arc<ScShimApi>> {
        loop {
            if let Some(s) = self.shim.lock().unwrap().clone() {
                return Some(s);
            }
            if self.sc_done() {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    pub fn set_sv_finished(&self) {
        self.sv_finished.store(true, Ordering::Release);
    }
}

static PENDING_LINK: Mutex<Option<Arc<ScLink>>> = Mutex::new(None);

/// Install the link the worker thread's `Simulator` will pick up.
pub fn store_pending_link(link: Arc<ScLink>) {
    *PENDING_LINK.lock().unwrap() = Some(link);
}

/// Take the pending link (called once from `Simulator::simulate`).
pub fn take_pending_link() -> Option<Arc<ScLink>> {
    PENDING_LINK.lock().unwrap().take()
}

// ── Shim ABI ────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct XscHostCb {
    pub ctx: *mut c_void,
    pub sv_step_exchange: Option<
        unsafe extern "C" fn(ctx: *mut c_void, cap: u64, quantum: u64) -> c_int,
    >,
}

macro_rules! shim_fns {
    ($($field:ident: $sig:ty = $name:literal),+ $(,)?) => {
        $(pub $field: $sig,)+
    };
}

/// Function table of a loaded model/cosim shared object.
pub struct ScShimApi {
    _lib: Arc<libloading::Library>,

    pub model_init: unsafe extern "C" fn(f64) -> c_int,
    pub model_finish: unsafe extern "C" fn(),
    pub signal_count: unsafe extern "C" fn() -> c_int,
    pub signal_name: unsafe extern "C" fn(c_int) -> *const c_char,
    pub signal_width: unsafe extern "C" fn(c_int) -> c_int,
    pub find_signal: unsafe extern "C" fn(*const c_char) -> c_int,
    pub write_export: unsafe extern "C" fn(c_int, *const u64, c_int),
    pub read_import: unsafe extern "C" fn(c_int, *mut u64, c_int),
    pub step: unsafe extern "C" fn(u64),
    pub time_now: unsafe extern "C" fn() -> u64,
    pub cosim_main: unsafe extern "C" fn(c_int, *mut *mut c_char, *const XscHostCb, f64) -> c_int,
    pub sv_wait_slice: unsafe extern "C" fn() -> c_int,
    pub sv_import_ptr: unsafe extern "C" fn(c_int) -> *const u64,
    pub sv_set_finished: unsafe extern "C" fn(),
    pub sv_cap: unsafe extern "C" fn() -> u64,
    pub has_pending: unsafe extern "C" fn() -> c_int,
}

impl ScShimApi {
    /// `dlopen` a model/cosim shared object and resolve the xsc_* ABI.
    pub fn load(path: &str) -> Result<Self, String> {
        unsafe {
            let lib = libloading::Library::new(path)
                .map_err(|e| format!("cannot load SystemC model '{path}': {e}"))?;
            let lib = Arc::new(lib);
            macro_rules! sym {
                ($name:literal, $t:ty) => {
                    *lib.get::<$t>(concat!($name, "\0").as_bytes())
                        .map_err(|e| {
                            format!("SystemC model '{path}' is missing '{}': {e}", $name)
                        })?
                };
            }
            let model_init = sym!("xsc_model_init", unsafe extern "C" fn(f64) -> c_int);
            let model_finish = sym!("xsc_model_finish", unsafe extern "C" fn());
            let signal_count = sym!("xsc_signal_count", unsafe extern "C" fn() -> c_int);
            let signal_name =
                sym!("xsc_signal_name", unsafe extern "C" fn(c_int) -> *const c_char);
            let signal_width = sym!("xsc_signal_width", unsafe extern "C" fn(c_int) -> c_int);
            let find_signal =
                sym!("xsc_find_signal", unsafe extern "C" fn(*const c_char) -> c_int);
            let write_export =
                sym!("xsc_write_export", unsafe extern "C" fn(c_int, *const u64, c_int));
            let read_import =
                sym!("xsc_read_import", unsafe extern "C" fn(c_int, *mut u64, c_int));
            let step = sym!("xsc_step", unsafe extern "C" fn(u64));
            let time_now = sym!("xsc_time_now", unsafe extern "C" fn() -> u64);
            let cosim_main = sym!(
                "xsc_cosim_main",
                unsafe extern "C" fn(c_int, *mut *mut c_char, *const XscHostCb, f64) -> c_int
            );
            let sv_wait_slice = sym!("xsc_sv_wait_slice", unsafe extern "C" fn() -> c_int);
            let sv_import_ptr =
                sym!("xsc_sv_import_ptr", unsafe extern "C" fn(c_int) -> *const u64);
            let sv_set_finished = sym!("xsc_sv_set_finished", unsafe extern "C" fn());
            let sv_cap = sym!("xsc_sv_cap", unsafe extern "C" fn() -> u64);
            let has_pending = sym!("xsc_has_pending", unsafe extern "C" fn() -> c_int);
            Ok(Self {
                _lib: lib,
                model_init,
                model_finish,
                signal_count,
                signal_name,
                signal_width,
                find_signal,
                write_export,
                read_import,
                step,
                time_now,
                cosim_main,
                sv_wait_slice,
                sv_import_ptr,
                sv_set_finished,
                sv_cap,
                has_pending,
            })
        }
    }
}

/// Compile the user's SystemC sources together with the xezim shim into a
/// model/cosim shared object, and load it. Returns the API.
pub fn build_and_load(opts: &ScOptions) -> Result<ScShimApi, String> {
    let sc_home = std::env::var("SYSTEMC_HOME")
        .map_err(|_| "SYSTEMC_HOME is not set (required for SystemC co-simulation; \
                     point it at a SystemC 2.3+/3.x installation)".to_string())?;
    let inc = format!("{sc_home}/include");
    if !std::path::Path::new(&inc).join("systemc").exists() {
        return Err(format!("SYSTEMC_HOME={sc_home} has no include/systemc"));
    }

    if !opts.sources.is_empty() || opts.libs.is_empty() {
        let out_dir = std::env::temp_dir().join(format!(
            "xezim-sc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ));
        std::fs::create_dir_all(&out_dir)
            .map_err(|e| format!("cannot create {out_dir:?}: {e}"))?;
        let so_path = out_dir.join("libxezim_sc_model.so");

        let mut cmd = std::process::Command::new("g++");
        cmd.arg("-shared")
            .arg("-fPIC")
            .arg("-std=c++17")
            .arg("-O1")
            .arg("-I")
            .arg(&inc)
            .arg("-I")
            .arg(shim_include_dir());
        for d in &opts.include_dirs {
            cmd.arg("-I").arg(d);
        }
        for f in &opts.cxxflags {
            cmd.arg(f);
        }
        for s in &opts.sources {
            cmd.arg(s);
        }
        cmd.arg(shim_source());
        let libdir = format!("{sc_home}/lib");
        cmd.arg("-L").arg(&libdir).arg("-lsystemc");
        // Also probe lib64 (some distributions install there).
        if std::path::Path::new(&format!("{sc_home}/lib64")).exists() {
            cmd.arg("-L").arg(format!("{sc_home}/lib64"));
        }
        cmd.arg(format!("-Wl,-rpath,{libdir}"))
            .arg("-o")
            .arg(&so_path);

        let out = cmd
            .output()
            .map_err(|e| format!("cannot run g++ for the SystemC model: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "SystemC model compilation failed:\n{}{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            ));
        }
        return ScShimApi::load(so_path.to_string_lossy().as_ref());
    }

    // Prebuilt model only.
    ScShimApi::load(&opts.libs[0])
}

// ── Bridge state inside the Simulator ───────────────────────────────────────

/// One SV<->SC signal crossing.
pub struct ScMapRow {
    /// SystemVerilog signal id in the simulator's signal table.
    pub sv_id: usize,
    /// Registry name on the SystemC side (for error messages).
    pub sc_name: String,
    pub dir: ScDir,
    /// Width in bits of the SystemVerilog signal.
    pub sv_width: u32,
    /// Registry index; resolved lazily (the SystemC registry exists only
    /// after the user's sc_main()/xsc_model_build() ran).
    pub sc_idx: Option<i32>,
}

/// A registry signal mirrored for waveform tracing.
pub struct ScTraceRow {
    pub name: String,
    pub width: u32,
    /// Last known value (updated at every exchange).
    pub mirror: Value,
    /// Prev value seen by each active waveform writer.
    pub vcd_code: Option<Arc<str>>,
    pub vcd_prev: Option<Value>,
    pub fst_id: Option<fst_writer::FstSignalId>,
    pub fst_prev: Option<Value>,
    pub xt_sid: Option<Arc<str>>,
    pub xt_prev: Option<Value>,
}

impl ScTraceRow {
    fn new(name: String, width: u32) -> Self {
        Self {
            name,
            width,
            mirror: Value::from_u64(0, width),
            vcd_code: None,
            vcd_prev: None,
            fst_id: None,
            fst_prev: None,
            xt_sid: None,
            xt_prev: None,
        }
    }

    /// true if any waveform writer is mirroring this row.
    pub fn traced(&self) -> bool {
        self.vcd_code.is_some() || self.fst_id.is_some() || self.xt_sid.is_some()
    }
}

/// Per-Simulator bridge state.
pub struct ScBridge {
    pub shim: Arc<ScShimApi>,
    pub rows: Vec<ScMapRow>,
    pub trace: Vec<ScTraceRow>,
    pub sv_top: bool,
}

impl ScBridge {
    /// SV top: load the model .so, run xsc_model_init(tick_s) (which calls
    /// the user's xsc_model_build), then enumerate the registry.
    pub fn init_sv_top(opts: &ScOptions, tick_s: f64) -> Result<Self, String> {
        let shim = Arc::new(build_and_load(opts)?);
        let rc = unsafe { (shim.model_init)(tick_s) };
        if rc != 0 {
            return Err(format!("xsc_model_init failed (code {rc})"));
        }
        let mut b = Self {
            shim,
            rows: Vec::new(),
            trace: Vec::new(),
            sv_top: true,
        };
        b.enumerate_registry()?;
        Ok(b)
    }

    /// SC top: the .so lives on the SystemC host thread; we only hold the
    /// published handle. Registry enumeration happens on first rendezvous.
    pub fn init_sc_top(shim: Arc<ScShimApi>) -> Result<Self, String> {
        Ok(Self {
            shim,
            rows: Vec::new(),
            trace: Vec::new(),
            sv_top: false,
        })
    }

    fn enumerate_registry(&mut self) -> Result<(), String> {
        unsafe {
            let n = (self.shim.signal_count)();
            if self.trace.is_empty() {
                for i in 0..n {
                    let name = CStr::from_ptr((self.shim.signal_name)(i))
                        .to_string_lossy()
                        .into_owned();
                    let w = (self.shim.signal_width)(i);
                    self.trace
                        .push(ScTraceRow::new(name, w.max(1) as u32));
                }
            }
        }
        Ok(())
    }

    /// Resolve lazily-bound registry indices; enumerates the trace rows on
    /// the first call (registry is stable after elaboration).
    pub fn ensure_ready(&mut self) {
        if self.trace.is_empty() {
            let _ = self.enumerate_registry();
        }
        for row in &mut self.rows {
            if row.sc_idx.is_none() {
                if let Ok(cname) = CString::new(row.sc_name.as_str()) {
                    let idx = unsafe { (self.shim.find_signal)(cname.as_ptr()) };
                    if idx >= 0 {
                        row.sc_idx = Some(idx);
                    }
                }
            }
        }
    }

    pub fn add_map(&mut self, sv_id: usize, sv_width: u32, sc_name: String, dir: ScDir) {
        self.rows.push(ScMapRow {
            sv_id,
            sc_name,
            dir,
            sv_width,
            sc_idx: None,
        });
        self.ensure_ready();
    }

    /// Pull every import-mapped value from the shim, appending
    /// (sv_id, word) pairs. The caller applies them to the signal table.
    pub fn collect_imports(&self, out: &mut Vec<(usize, u64)>) {
        for row in &self.rows {
            if row.dir != ScDir::Import {
                continue;
            }
            let Some(idx) = row.sc_idx else { continue };
            let mut buf = [0u64; 1];
            unsafe { (self.shim.read_import)(idx, buf.as_mut_ptr(), 1) };
            out.push((row.sv_id, buf[0]));
        }
    }

    /// Refresh the trace mirror from the registry (all signals, regardless
    /// of mapping — the bridge is the window into the SystemC design).
    pub fn update_mirrors(&mut self) {
        for (i, row) in self.trace.iter_mut().enumerate() {
            let mut buf = [0u64; 1];
            unsafe { (self.shim.read_import)(i as i32, buf.as_mut_ptr(), 1) };
            // Mask to the declared width for a clean display.
            let w = row.width as usize;
            let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            row.mirror = Value::from_u64(buf[0] & mask, row.width);
        }
    }
}

// ── SC-top host thread ──────────────────────────────────────────────────────

/// Body of the SystemC host thread (SC top). Compiles + loads the cosim .so,
/// publishes the shim, and blocks in `xsc_cosim_main` until `sc_main`
/// returns.
pub fn run_sc_top_thread(link: Arc<ScLink>) {
    let opts = match sc_options() {
        Some(o) => o,
        None => {
            link.sc_done.store(true, Ordering::Release);
            return;
        }
    };

    let Some(tick_s) = link.wait_tick_s() else {
        // SV side never came up; nothing to co-simulate with.
        link.sc_done.store(true, Ordering::Release);
        return;
    };

    match build_and_load(&opts) {
        Ok(api) => {
            let shim = Arc::new(api);
            link.publish_shim(shim.clone());
            let mut argv = [b"xezim\0".as_ptr() as *mut c_char];
            let rc = unsafe {
                (shim.cosim_main)(1, argv.as_mut_ptr() as *mut *mut c_char, std::ptr::null(), tick_s)
            };
            if rc != 0 {
                eprintln!("[xezim-sc] xsc_cosim_main returned {rc}");
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
        }
    }
    link.sc_done.store(true, Ordering::Release);
}
