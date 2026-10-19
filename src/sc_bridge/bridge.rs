//! Per-`Simulator` SystemC bridge runtime state.

use std::ffi::CStr;
use std::sync::Arc;

use xezim_core::Value;

use super::build::build_and_load;
use super::link::ScShimApi;
use super::options::ScDir;
use super::options::ScOptions;
use super::wrapper::WrapperBundle;

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
    /// Parameter passing (SV → SC): the SV top's parameters are handed to the
    /// SHIM *before* the model ctor runs so a pure-SystemC module can read
    /// them with `sc_get_*_param`.
    pub fn init_sv_top(
        opts: &ScOptions,
        tick_s: f64,
        bundle: &WrapperBundle,
        params: &[(String, String)],
    ) -> Result<Self, String> {
        let shim = Arc::new(build_and_load(opts, bundle)?);
        shim.set_params(params);
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
                    self.trace.push(ScTraceRow::new(name, w.max(1) as u32));
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
                if let Ok(cname) = std::ffi::CString::new(row.sc_name.as_str()) {
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
            // Import and Auto rows both pull the SystemC value back.
            if row.dir == ScDir::Export {
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