//! The link shared between the two kernels and the shim ABI table.

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use super::wrapper::WrapperBundle;

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
    /// Auto-derived wrapper artifacts produced by the SV worker (top ports are
    /// known only there) and awaited by the SystemC host thread before it
    /// compiles the model.
    wrapper_mu: Mutex<Option<WrapperBundle>>,
    wrapper_cv: Condvar,
    wrapper_ready: AtomicBool,
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
            wrapper_mu: Mutex::new(None),
            wrapper_cv: Condvar::new(),
            wrapper_ready: AtomicBool::new(false),
            sv_finished: AtomicBool::new(false),
        }
    }

    /// Publish the auto-derived wrapper (SV worker → SC host thread). Idempotent.
    pub fn publish_wrapper(&self, bundle: WrapperBundle) {
        let mut g = self.wrapper_mu.lock().unwrap();
        if g.is_none() {
            *g = Some(bundle);
        }
        self.wrapper_ready.store(true, Ordering::Release);
        self.wrapper_cv.notify_all();
    }

    /// Wait for the SV worker to publish the wrapper (only used by SC-top
    /// builds). Returns an empty bundle if the worker finished first.
    pub fn await_wrapper(&self) -> Option<WrapperBundle> {
        let g = self.wrapper_mu.lock().unwrap();
        let mut guard = g;
        while !self.wrapper_ready.load(Ordering::Acquire) && !self.sc_done() {
            let (mu, _) = self
                .wrapper_cv
                .wait_timeout(guard, std::time::Duration::from_millis(100))
                .unwrap();
            guard = mu;
        }
        guard.take()
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
        self.tick_s_bits.store(tick_s.to_bits(), Ordering::Release);
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

/// Host callback invoked once per slice by the shim's slice loop (SC top).
#[repr(C)]
pub struct XscHostCb {
    pub ctx: *mut c_void,
    pub sv_step_exchange: Option<
        unsafe extern "C" fn(ctx: *mut c_void, cap: u64, quantum: u64) -> c_int,
    >,
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
    pub step_mode_b: unsafe extern "C" fn(u64),
    pub time_now: unsafe extern "C" fn() -> u64,
    pub cosim_main: unsafe extern "C" fn(c_int, *mut *mut c_char, *const XscHostCb, f64) -> c_int,
    pub sv_wait_slice: unsafe extern "C" fn() -> c_int,
    pub sv_import_ptr: unsafe extern "C" fn(c_int) -> *const u64,
    pub sv_set_finished: unsafe extern "C" fn(),
    pub sv_cap: unsafe extern "C" fn() -> u64,
    pub has_pending: unsafe extern "C" fn() -> c_int,
    pub param_set_count: unsafe extern "C" fn(c_int),
    pub param_set_at: unsafe extern "C" fn(c_int, *const c_char, *const c_char),
    pub param_count: unsafe extern "C" fn() -> c_int,
    pub param_name: unsafe extern "C" fn(c_int) -> *const c_char,
    pub param_value: unsafe extern "C" fn(c_int) -> *const c_char,
}

impl ScShimApi {
    /// Fetch every name=value parameter the SC side registered.
    pub fn dump_params(&self) -> Vec<(String, String)> {
        unsafe {
            let n = (self.param_count)();
            let mut out = Vec::with_capacity(n.max(0) as usize);
            for i in 0..n {
                let n_p = (self.param_name)(i);
                let v_p = (self.param_value)(i);
                if n_p.is_null() { continue; }
                let name = CStr::from_ptr(n_p).to_string_lossy().into_owned();
                let value = if v_p.is_null() { String::new() } else {
                    CStr::from_ptr(v_p).to_string_lossy().into_owned()
                };
                out.push((name, value));
            }
            out
        }
    }

    /// Replace the SC-side parameter table with `params` (name -> value).
    /// The SC module (SV-top mode) reads these with `sc_get_*_param`.
    pub fn set_params(&self, params: &[(String, String)]) {
        unsafe {
            (self.param_set_count)(params.len() as c_int);
            for (i, (name, value)) in params.iter().enumerate() {
                let n = std::ffi::CString::new(name.as_str()).unwrap_or_default();
                let v = std::ffi::CString::new(value.as_str()).unwrap_or_default();
                (self.param_set_at)(
                    i as c_int,
                    n.as_ptr(),
                    v.as_ptr(),
                );
            }
        }
    }
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
            #[allow(non_snake_case)]
            let step_mode_b = sym!("xsc_step_mode_b", unsafe extern "C" fn(u64));
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
            let param_set_count = sym!("xsc_set_param_count", unsafe extern "C" fn(c_int));
            let param_set_at = sym!(
                "xsc_set_param_at",
                unsafe extern "C" fn(c_int, *const c_char, *const c_char)
            );
            let param_count = sym!("xsc_param_count", unsafe extern "C" fn() -> c_int);
            let param_name = sym!("xsc_param_name", unsafe extern "C" fn(c_int) -> *const c_char);
            let param_value = sym!("xsc_param_value", unsafe extern "C" fn(c_int) -> *const c_char);
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
                step_mode_b,
                time_now,
                cosim_main,
                sv_wait_slice,
                sv_import_ptr,
                sv_set_finished,
                sv_cap,
                has_pending,
                param_set_count,
                param_set_at,
                param_count,
                param_name,
                param_value,
            })
        }
    }
}