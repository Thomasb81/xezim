//! Per-`Simulator` SystemC bridge runtime state.

use std::ffi::CStr;
use std::sync::Arc;

use xezim_core::value::LogicBit;
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

impl ScMapRow {
    /// Number of 64-bit words this crossing transfers across the bridge
    /// (ceil(width/64); the shim registry is word-granular and the SV value
    /// engine exposes at most a two-word / u128 window at the boundary).
    pub fn nwords(&self) -> usize {
        (self.sv_width.max(1) as usize + 63) / 64
    }

    pub fn nwords_of(width: u32) -> usize {
        (width.max(1) as usize + 63) / 64
    }
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

/// Extract `(val_words, xz_words)` (LSB-first `u64` slices) from a 4-state `Value`.
pub fn value_to_planes(v: &Value, width: u32) -> (Vec<u64>, Vec<u64>) {
    let w = width.max(1);
    let n = ScMapRow::nwords_of(w);
    let mut val = vec![0u64; n];
    let mut xz = vec![0u64; n];
    if w <= 64 {
        let (vb, xb) = v.raw_bits();
        let mask = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
        val[0] = vb & mask;
        xz[0] = xb & mask;
        return (val, xz);
    }
    if !v.has_xz() && v.words_if_clean(&mut val) {
        let rem = w % 64;
        if rem != 0 {
            val[n - 1] &= (1u64 << rem) - 1;
        }
        return (val, xz);
    }
    for i in 0..(w as usize) {
        let code = v.get_bit_code(i);
        if (code & 1) != 0 {
            val[i / 64] |= 1u64 << (i % 64);
        }
        if (code & 2) != 0 {
            xz[i / 64] |= 1u64 << (i % 64);
        }
    }
    (val, xz)
}

/// Build a 4-state `Value` from `(val_words, xz_words)`.
/// Encoding: 0=(v=0,xz=0), 1=(v=1,xz=0), X=(v=0,xz=1), Z=(v=1,xz=1).
pub fn planes_to_value(val: &[u64], xz: &[u64], width: u32) -> Value {
    let w = width.max(1);
    if w <= 64 {
        let mask = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
        let vb = val.first().copied().unwrap_or(0) & mask;
        let xb = xz.first().copied().unwrap_or(0) & mask;
        return Value::from_inline(vb, xb, w);
    }
    let mut out = Value::from_words(val, w);
    let n = ScMapRow::nwords_of(w);
    for wi in 0..n {
        let xw = xz.get(wi).copied().unwrap_or(0);
        if xw == 0 {
            continue;
        }
        let vw = val.get(wi).copied().unwrap_or(0);
        for bi in 0..64 {
            let bit_idx = wi * 64 + bi;
            if bit_idx >= w as usize {
                break;
            }
            if ((xw >> bi) & 1) != 0 {
                let lb = if ((vw >> bi) & 1) != 0 {
                    LogicBit::Z
                } else {
                    LogicBit::X
                };
                out.set_bit(bit_idx, lb);
            }
        }
    }
    out
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

    pub fn enumerate_registry(&mut self) -> Result<(), String> {
        unsafe {
            let n = (self.shim.signal_count)();
            let start = self.trace.len() as i32;
            for i in start..n {
                let name = CStr::from_ptr((self.shim.signal_name)(i))
                    .to_string_lossy()
                    .into_owned();
                let w = (self.shim.signal_width)(i);
                self.trace.push(ScTraceRow::new(name, w.max(1) as u32));
            }
        }
        Ok(())
    }

    /// Resolve lazily-bound registry indices; enumerates the trace rows on
    /// the first call (or after bootstrap spawns additional leaves).
    pub fn ensure_ready(&mut self) {
        let _ = self.enumerate_registry();
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

    /// Pull every import-mapped 4-state value from the shim, appending
    /// `(sv_id, val_words, xz_words)` triples.
    pub fn collect_imports(&self, out: &mut Vec<(usize, Vec<u64>, Vec<u64>)>) {
        for row in &self.rows {
            // Import and Auto rows both pull the SystemC value back.
            if row.dir == ScDir::Export {
                continue;
            }
            let Some(idx) = row.sc_idx else { continue };
            let n = row.nwords();
            let mut val = vec![0u64; n];
            let mut xz = vec![0u64; n];
            self.shim.read_import_planes(idx, &mut val, &mut xz);
            out.push((row.sv_id, val, xz));
        }
    }

    /// Refresh the trace mirror from the registry (all signals, regardless
    /// of mapping — the bridge is the window into the SystemC design).
    pub fn update_mirrors(&mut self) {
        for (i, row) in self.trace.iter_mut().enumerate() {
            let n = ScMapRow::nwords_of(row.width);
            let mut val = vec![0u64; n];
            let mut xz = vec![0u64; n];
            self.shim.read_import_planes(i as i32, &mut val, &mut xz);
            let rem = row.width % 64;
            if rem != 0 {
                let mask = (1u64 << rem) - 1;
                if let Some(hw) = val.last_mut() {
                    *hw &= mask;
                }
                if let Some(hw) = xz.last_mut() {
                    *hw &= mask;
                }
            }
            row.mirror = planes_to_value(&val, &xz, row.width);
        }
    }
}