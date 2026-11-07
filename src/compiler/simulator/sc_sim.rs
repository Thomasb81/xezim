//! SystemC co-simulation (`--sc-source`/`--sc-lib`): the `impl Simulator`
//! methods that drive the `sc_bridge` from the SV side — SC-top rendezvous,
//! SV-top `$sc_step`/`$sc_*` task action, Mode B auto-stepping, and the
//! auto-derived wrapper / instance-port mapping. Runtime-optional: with no
//! `sc_bridge` present every method is a no-op, so nothing here depends on
//! SystemC being installed.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

/// Render a SystemVerilog parameter value to the text form the SystemC side's
/// `sc_get_param` family expects (mirrors the reference simulators' `vlog_to_sc`
/// parameter passing).
pub(super) fn sc_param_value_text(v: &xezim_core::Value) -> String {
    if v.is_real {
        let d = v.to_f64();
        if d == d.trunc() && d.abs() < 1e9 {
            format!("{:.0}", d)
        } else {
            format!("{}", d)
        }
    } else if v.has_xz() && v.width > 0 {
        // A vector containing unknown/high-impedance bits (e.g. `8'b001100xz`)
        // is neither a plain decimal (to_dec_string collapses X/Z -> X) nor a
        // printable string (to_sv_string masks X/Z to a printable '0' byte,
        // which the string check below would wrongly accept). Match the
        // reference (sc_get_string_param with format_char 'b') by handing SC
        // the width-padded binary string that preserves the X/Z verbatim.
        v.to_bin()
    } else if v.width > 0 && v.width % 8 == 0 {
        let s = v.to_sv_string();
        if !s.is_empty() && s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return format!("\"{}\"", s);
        }
        v.to_dec_string()
    } else {
        v.to_dec_string()
    }
}

impl Simulator {

    // ═════════════════════════════════════════════════════════
    // SystemC co-simulation (runtime-optional; see docs/systemc-cosim.md)
    // ═════════════════════════════════════════════════════════

    /// Sanitize a SystemC registry name into a waveform leaf name: the last
    /// path component, with characters outside `[A-Za-z0-9_$]` replaced.
    pub(super) fn sc_trace_leaf(name: &str) -> String {
        let leaf = name.rsplit(['/', '.']).next().unwrap_or(name);
        leaf.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '_' | '$') {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// `$sc_export_sig(sv_signal, "sc_name")` /
    /// `$sc_import_sig(sv_signal, "sc_name")`: attach a SystemVerilog signal
    /// to a SystemC registry signal (SV top). Export = SV drives SC.
    pub(crate) fn exec_sc_map_task(&mut self, name: &str, args: &[Expression]) {
        // Call convention: ("registry_name", signal) — the string first,
        // matching $dumpfile-style system tasks.
        if args.len() != 2 {
            eprintln!("Warning: {} expects (\"registry_name\", signal)", name);
            return;
        }
        let sc_name = self.system_string_arg(&args[0]);
        let hier = match &args[1].kind {
            ExprKind::Ident(h) => h.clone(),
            _ => {
                eprintln!("Warning: {} second argument must be a signal identifier", name);
                return;
            }
        };
        let full = self.resolve_hier_name(&hier).into_owned();
        self.sync_table_to_hashmap();
        let Some(&sv_id) = self.signal_name_to_id.get(full.as_str()) else {
            eprintln!("Warning: {}: '{}' does not name a signal", name, full);
            return;
        };
        let width = self.signal_table[sv_id].width;
        if width > 64 {
            eprintln!(
                "Error: {}: signals wider than 64 bits are not supported",
                name
            );
            self.finished = true;
            return;
        }
        let dir = if name == "$sc_export_sig" {
            crate::sc_bridge::ScDir::Export
        } else {
            crate::sc_bridge::ScDir::Import
        };
        let Some(b) = self.sc_bridge.as_mut() else {
            eprintln!(
                "Error: {} used without a SystemC model — pass --sc-source or --sc-lib",
                name
            );
            self.finished = true;
            return;
        };
        // An Auto (name-matched) row already covers any direction; also
        // dedup an exact repeat so double tasks are harmless.
        if b.rows.iter().any(|r| {
            r.sv_id == sv_id
                && r.sc_name == sc_name
                && (r.dir == dir || r.dir == crate::sc_bridge::ScDir::Auto)
        }) {
            return;
        }
        b.add_map(sv_id, width, sc_name, dir);
    }

    /// Name-based auto-attachment — the SystemC analogue of simulating an
    /// uninstantiated root module: every registry signal whose name matches
    /// an SV signal name is attached automatically, synced both ways each
    /// exchange (ScDir::Auto). Explicit rows ($sc_export_sig / $sc_import_sig)
    /// keep precedence; a wider-than-128-bit match is skipped with a warning
    /// rather than aborting the run.
    pub(crate) fn sc_auto_attach(&mut self, b: &mut crate::sc_bridge::ScBridge) {
        self.sync_table_to_hashmap();
        b.ensure_ready();
        let names: Vec<(String, u32)> = b
            .trace
            .iter()
            .map(|r| (r.name.clone(), r.width))
            .collect();
        let mapped: std::collections::HashSet<String> =
            b.rows.iter().map(|r| r.sc_name.clone()).collect();
        for (name, w) in names {
            if mapped.contains(name.as_str()) {
                continue;
            }
            let Some(&id) = self.signal_name_to_id.get(name.as_str()) else {
                continue;
            };
            if w > 128 {
                eprintln!(
                    "Warning: SystemC registry signal '{}' ({} bits) exceeds the \
                     128-bit crossing limit; not auto-attached",
                    name, w
                );
                continue;
            }
            // Direction-aware attach: an SV input port receives from SystemC
            // (Import), an SV output port drives SystemC (Export), and an
            // inout is genuinely bidirectional (Auto). Without this, an SV
            // INPUT would be staged back by the SV kernel (its stale initial
            // value) and clobber the value the SystemC side drove before the
            // SystemC write ever reached SV.
            let dir = match self.vpi_port_directions.get(name.as_str()) {
                Some(PortDirection::Input) => crate::sc_bridge::ScDir::Import,
                Some(PortDirection::Output) => crate::sc_bridge::ScDir::Export,
                _ => crate::sc_bridge::ScDir::Auto,
            };
            b.add_map(id, w, name, dir);
        }
    }

    /// Mode B: discover the netlist SC-leaf instance prefixes, e.g. `u0`/`u1`
    /// for two `inc_leaf u0(...)/u1(...)` sites, plus the single-instance
    /// `u`. Reuses the same elaboration-agenda the port-application walk reads
    /// (`port_aliases`, `input_port_actuals`, `continuous_assigns`,
    /// `signal_name_to_id`) so the generated leaf model can be built with one
    /// SC object per instance before the bridge maps them.
    pub(crate) fn sc_leaf_instances(&self) -> Vec<String> {
        use crate::sc_bridge::sc_module_ports;
        let Some(opts) = crate::sc_bridge::sc_options() else {
            return Vec::new();
        };
        let Some(mname) = opts.sc_module.clone() else {
            return Vec::new();
        };
        let ports = sc_module_ports(&mname, &opts.sources);
        if ports.is_empty() {
            return Vec::new();
        }
        let scnames: std::collections::HashSet<&str> =
            ports.iter().map(|p| p.name.as_str()).collect();
        fn flat_path(e: &crate::ast::expr::Expression) -> Option<String> {
            if let crate::ast::expr::ExprKind::Ident(h) = &e.kind {
                Some(
                    h.path
                        .iter()
                        .map(|s| s.name.name.to_string())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            } else {
                None
            }
        }
        // net_of: `<inst>.<port>` → the SV net carrying that port's value
        // (see `sc_attach_instance_ports` for the three routing rules).
        let mut net_of: std::collections::HashMap<String, String> = Default::default();
        for (key, actual) in &self.module.port_aliases {
            net_of.insert(key.clone(), actual.clone());
        }
        // In main, `input_port_actuals` entries are `(<inst>.<port> formal,
        // actual expr, two_state)` — the formal is the FULL path (e.g.
        // `u.clock`), not the bare instance prefix. Key `net_of` directly by
        // the formal path so each SC input port resolves to its connected SV
        // net by NAME (not positionally, which the old branch's bare-instance
        // keys implied).
        for port in &self.module.input_port_actuals {
            if let Some(a) = flat_path(port.actual()) {
                net_of.insert(port.formal().to_string(), a);
            }
        }
        for ca in &self.module.continuous_assigns {
            if let (Some(lhs), Some(rhs)) = (flat_path(&ca.lhs), flat_path(&ca.rhs)) {
                if let Some(idx) = lhs.rfind('.') {
                    let por = &lhs[idx + 1..];
                    if scnames.contains(por) && !por.contains('.') {
                        net_of.entry(lhs.clone()).or_insert_with(|| rhs.clone());
                    }
                }
            }
        }
        // Candidate instance prefixes: any `<inst>.<scport>` net name.
        let mut prefixes: std::collections::BTreeSet<String> = Default::default();
        for key in net_of.keys().map(String::as_str).chain(
            self.signal_name_to_id.keys().map(|k| k.as_ref()),
        ) {
            if let Some(idx) = key.rfind('.') {
                let (pre, por) = key.split_at(idx);
                let por = &por[1..];
                if scnames.contains(por) && !pre.contains('.') && !pre.is_empty() {
                    prefixes.insert(pre.to_string());
                }
            }
        }
        prefixes.into_iter().collect()
    }

    /// Mode B: bind each pure-SystemC *leaf* instance's port signals to the
    /// `u.dout` for `ringbuf u(.data_out(q))`). For each instance we find in
    /// the netlist that carries ALL of the SC module's ports, we attach its
    /// port signals to the registry under the bare port names.
    ///
    /// Direction: `ScPort.dir` is the SC module's own port kind
    /// (`sc_in` → Import of SC's view = SV exports into SC; `sc_out` → SC
    /// imports into SV). At the row level an SC `sc_in` (SC *reads* SV) is an
    /// **Export** row (SV→SC) and an SC `sc_out` (SC *drives* SV) is an
    /// **Import** row (SC→SV); `sc_inout`/`sc_signal` stay Auto. That is the
    /// complement of `ScPort.dir`.
    pub(crate) fn sc_attach_instance_ports(&mut self, b: &mut crate::sc_bridge::ScBridge) {
        use crate::sc_bridge::{sc_module_ports, ScDir, ScPort};
        let Some(opts) = crate::sc_bridge::sc_options() else {
            return;
        };
        let Some(mname) = opts.sc_module.clone() else {
            return;
        };
        // Reflect the SC module's ports from its C++ source.
        let ports = sc_module_ports(&mname, &opts.sources);
        if ports.is_empty() {
            return;
        }
        // Discover SC leaf instances. A leaf's SV-side values live on the
        // nets at its instantiation site. The elaborator handles a port
        // connection as follows:
        //   * OUTPUT whole-net actual → `port_aliases` entry `<inst>.<port>`
        //     ↦ actual net (u.dout → dout).
        //   * INPUT whole-net actual → an implicit continuous assign
        //     `assign <inst>.<port> = <actual>` and the port net is NOT
        //     materialised in the flat signal table (it collapses into the
        //     actual). We recover the actual from that assign's RHS.
        //   * bit-select / concat actual → the placeholder's own port signal
        //     `<inst>.<port>` stays in the flat signal table.
        // Every SC port must map to exactly one SV net by one of these three
        // routes; an instance is genuine only when ALL ports resolve.
        let scnames: std::collections::HashSet<&str> =
            ports.iter().map(|p| p.name.as_str()).collect();
        fn flat_path(e: &crate::ast::expr::Expression) -> Option<String> {
            if let crate::ast::expr::ExprKind::Ident(h) = &e.kind {
                Some(
                    h.path
                        .iter()
                        .map(|s| s.name.name.to_string())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            } else {
                None
            }
        }
        // net_of: `<inst>.<port>` → the SV net carrying that port's value.
        //   * OUTPUT whole-net actual → `port_aliases` (u.dout → dout).
        //   * INPUT whole-net actual → the port net is *elided* and the value
        //     lives on the actual; the elaborator records (inst, actual) in
        //     `input_port_actuals` in the placeholder's input-port order.
        //   * bit-select / concat actual → the placeholder's own port signal
        //     `<inst>.<port>` stays in the signal table.
        let mut net_of: std::collections::HashMap<String, String> = Default::default();
        for (key, actual) in &self.module.port_aliases {
            net_of.insert(key.clone(), actual.clone());
        }
        // The reflected inputs are recorded once per `<inst>.<port>` formal in
        // `input_port_actuals` (main keys it by the FULL path, not the bare
        // instance). Key `net_of` directly by that formal path so each SC input
        // port resolves to its connected SV net by NAME. (`sc_inout` is not
        // elided.)
        for port in &self.module.input_port_actuals {
            if let Some(a) = flat_path(port.actual()) {
                net_of.insert(port.formal().to_string(), a);
            }
        }
        // Route B: non-elided inputs appear as `assign <inst>.<port> = <net>`.
        for ca in &self.module.continuous_assigns {
            if let (Some(lhs), Some(rhs)) = (flat_path(&ca.lhs), flat_path(&ca.rhs)) {
                if let Some(idx) = lhs.rfind('.') {
                    let por = &lhs[idx + 1..];
                    if scnames.contains(por) && !por.contains('.') {
                        net_of.entry(lhs.clone()).or_insert_with(|| rhs.clone());
                    }
                }
            }
        }
        // Candidate instance prefixes: discovered once, shared with the
        // model-generation step so each site gets an independent SC object.
        let prefixes = self.sc_leaf_instances();
        for inst in &prefixes {
            // A genuine leaf only when EVERY SC port resolves to an SV net
            // underneath this instance.
            let mut all_ok = true;
            let mut bindings: Vec<(usize, &ScPort)> = Vec::new();
            for p in &ports {
                let sv_name = format!("{}.{}", inst, p.name);
                // Resolve: input-assign/alias net, else the placeholder's own
                // port signal (bit-select/concat actuals).
                let net = net_of
                    .get(sv_name.as_str())
                    .cloned()
                    .unwrap_or_else(|| sv_name.clone());
                let Some(&id) = self.signal_name_to_id.get(net.as_str()) else {
                    all_ok = false;
                    break;
                };
                bindings.push((id, p));
            }
            if !all_ok {
                continue;
            }
            // A genuine SC leaf is present: enable automatic per-time-step
            // exchange and restart the exchange-time baseline at t=0.
            self.sc_mode_b = true;
            self.sc_last_exchange_time = 0;
            for (id, p) in bindings {
                if p.width > 128 {
                    eprintln!(
                        "Warning: SystemC leaf port '{}' ({} bits) exceeds the 128-bit \
crossing limit; not attached",
                        p.name, p.width
                    );
                    continue;
                }
                // Complement mapping: SC sc_in → Export (SV→SC), sc_out →
                // Import (SC→SV), sc_inout → Auto.
                let dir = match p.dir {
                    ScDir::Import => ScDir::Export,
                    ScDir::Export => ScDir::Import,
                    ScDir::Auto => ScDir::Auto,
                };
                // Registry key is per-instance (`<inst>.<port>`), matching the
                // generated model's per-instance `xsc_bind`, so distinct
                // netlist sites do not collapse onto one shared SC signal.
                b.add_map(id, p.width, format!("{}.{}", inst, p.name), dir);
            }
        }
    }

    /// Collect the systemVerilog top module's ports (name, direction, width)
    /// for auto-deriving a SystemC counterpart. Port directions live in
    /// `vpi_port_directions` (keyed by interned signal name); widths resolve
    /// through the signal table.
    pub(crate) fn collect_top_ports(&mut self) -> Vec<crate::sc_bridge::TopPort> {
        self.sync_table_to_hashmap();
        let mut out = Vec::new();
        for (name, dir) in self.vpi_port_directions.iter() {
            let Some(&id) = self.signal_name_to_id.get(name.as_ref()) else {
                continue;
            };
            let width = self.signal_table[id].width;
            let sd = match dir {
                PortDirection::Input => crate::sc_bridge::ScPortDir::In,
                PortDirection::Output => crate::sc_bridge::ScPortDir::Out,
                PortDirection::Inout | PortDirection::Ref => crate::sc_bridge::ScPortDir::Inout,
            };
            out.push(crate::sc_bridge::TopPort {
                name: name.to_string(),
                dir: sd,
                width,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Stage export-mapped SV signals into the shim, returning
    /// (idx, words) pairs (1..2 words depending on crossing width).
    fn sc_stage_exports(&mut self) -> Vec<(i32, Vec<u64>)> {
        let Some(b) = self.sc_bridge.as_mut() else {
            return Vec::new();
        };
        // $sc_export_sig mappings created via exec_sc_map_task carry a lazy
        // sc_idx; resolve any that are still pending before the first exchange.
        b.ensure_ready();
        let mut out = Vec::new();
        for row in &b.rows {
            // Export and Auto rows both push the SV value into SystemC.
            if row.dir == crate::sc_bridge::ScDir::Import {
                continue;
            }
            if let Some(idx) = row.sc_idx {
                let (v, _x) = self.signal_table[row.sv_id].bits128();
                let n = row.nwords();
                let mut words = vec![0u64; n];
                words[0] = v as u64;
                if n > 1 {
                    words[1] = (v >> 64) as u64;
                }
                out.push((idx, words));
            }
        }
        out
    }

    /// Apply pulled import values into the signal table with full write
    /// bookkeeping (settle marks, dirty tracking, edge detection).
    fn sc_apply_imports(&mut self, imports: Vec<(usize, Vec<u64>)>) {
        for (sv_id, words) in imports {
            let w_bits = self.signal_table[sv_id].width;
            // Mask the top word to the declared width.
            let mut ws = words;
            let top = w_bits % 64;
            if let Some(hw) = ws.last_mut() {
                if top != 0 {
                    *hw &= (1u64 << top) - 1;
                }
            }
            // Two-state v1: the C++ readers already map sc_lv X/Z to 0.
            self.ts_store_wide(sv_id, &ws);
        }
    }

    /// `$sc_step(n)` (SV top): push exports, advance the SystemC kernel
    /// exactly n ticks, pull imports back. SystemC time after the call is
    /// exactly n*step earlier than after $sc_step(n-1) — exact accounting.
    pub(crate) fn sc_step_sv_top(&mut self, ticks: u64) {
        if self.sc_bridge.is_none() {
            eprintln!(
                "Error: $sc_step used without a SystemC model — pass --sc-source or --sc-lib"
            );
            self.finished = true;
            return;
        }
        let exports = self.sc_stage_exports();
        {
            let b = self.sc_bridge.as_ref().unwrap();
            for (idx, words) in &exports {
                unsafe { (b.shim.write_export)(*idx, words.as_ptr(), words.len() as i32) };
            }
            unsafe { (b.shim.step)(ticks) };
        }
        let mut imports = Vec::new();
        self.sc_bridge
            .as_ref()
            .unwrap()
            .collect_imports(&mut imports);
        if let Some(b) = self.sc_bridge.as_mut() {
            b.update_mirrors();
        }
        self.sc_apply_imports(imports);
    }

    /// Mode B exchange: the SC kernel is co-scheduled with SV time, so it is
    /// advanced first (with the previously-applied SV inputs), then the new SV
    /// inputs are applied at the endpoint, then deltas drain so an edge-
    /// triggered SC output commits within the SAME SV slot (see xsc_step_mode_b).
    /// The very first call (ticks == 0) merely elaborates SC and, with the
    /// t=0 exports applied, exposes the SC module's initialized outputs on the
    /// SV netlist at time 0.
    pub(crate) fn sc_step_sv_top_mode_b(&mut self, ticks: u64) {
        if self.sc_bridge.is_none() {
            self.finished = true;
            return;
        }
        let exports = self.sc_stage_exports();
        {
            let b = self.sc_bridge.as_ref().unwrap();
            for (idx, words) in &exports {
                unsafe { (b.shim.write_export)(*idx, words.as_ptr(), words.len() as i32) };
            }
            unsafe { (b.shim.step_mode_b)(ticks) };
        }
        let mut imports = Vec::new();
        self.sc_bridge
            .as_ref()
            .unwrap()
            .collect_imports(&mut imports);
        if let Some(b) = self.sc_bridge.as_mut() {
            b.update_mirrors();
        }
        self.sc_apply_imports(imports);
    }

    /// One SC-top exchange: stage exports, block until the SystemC slice
    /// completes, apply its imports, refresh the trace mirrors. Returns
    /// false when the SystemC side is done (sc_main returned / failed).
    /// True when the SystemC kernel can still produce future activity (or
    /// has not started yet). Both-kernels-quiescent means the co-simulation is
    /// over: parked SV waiters can never be resumed.
    fn sc_side_alive(&self) -> bool {
        match self.sc_bridge.as_ref() {
            Some(b) => unsafe { (b.shim.has_pending)() != 0 },
            None => false,
        }
    }

    /// Whether the SystemC side still expects work: either the SystemC kernel
    /// has projected future activity (a not-yet-elaborated/not-yet-finished
    /// kernel reports pending), or the SV side has staged an export value the
    /// SystemC side hasn't consumed yet. The second clause is what lets an
    /// SC-top exchange deliver an SV output whose combinational fanout settled
    /// only after the last exchange — without it, a quiet SystemC side would
    /// end the run and the SV output would never reach sc_main.
    ///
    /// Additionally, in the auto-derive SC-top flow the SystemC master's
    /// `sc_main` drives the DUT imperatively (write + `sc_start()`), so between
    /// the master's steps the SC kernel is quiescent yet more steps are coming.
    /// While `sc_main` has not returned (`!sc_done`) the SV kernel must park on
    /// the link rather than treat the momentary quiescence as the end, otherwise
    /// an imported clock edge written later never reaches the SV kernel.
    pub(crate) fn sc_exchange_alive(&mut self) -> bool {
        if self.sc_side_alive() {
            return true;
        }
        if let Some(link) = self.sc_link.as_ref() {
            if !link.sc_done() {
                // SC master still inside sc_main; it may call sc_start() again,
                // so keep the SV side parked awaiting the next step.
                return true;
            }
        }
        // NV: any Export/Auto row whose current value differs from what we
        // last staged still needs one more exchange to reach SystemC.
        let Some(b) = self.sc_bridge.as_ref() else { return false };
        for (idx, prev) in &self.sc_last_exports {
            for row in &b.rows {
                if row.sv_id == *idx
                    && row.dir != crate::sc_bridge::ScDir::Import
                    && row.sc_idx.is_some()
                {
                    let (v, _x) = self.signal_table[*idx].bits128();
                    let n = row.nwords();
                    let mut cur = vec![0u64; n];
                    cur[0] = v as u64;
                    if n > 1 {
                        cur[1] = (v >> 64) as u64;
                    }
                    if cur != *prev {
                        return true;
                    }
                    break;
                }
            }
        }
        false
    }

    pub(crate) fn sc_rendezvous(&mut self) -> bool {
        // Remove the debug print and record the staged snapshot below.
        let Some(link) = self.sc_link.clone() else {
            return true;
        };
        if link.sc_done() {
            return false;
        }
        if self.sc_bridge.is_none() {
            // No model was ever loaded (its failure was reported at startup);
            // park until the host thread flags completion.
            while !link.sc_done() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            return false;
        }
        let exports = self.sc_stage_exports();
        // Record what we staged this exchange for pending-export detection.
        self.sc_last_exports = exports
            .iter()
            .map(|(idx, v)| (*idx as usize, v.clone()))
            .collect();
        // 1. Publish this kernel's exports for the slice.
        {
            let b = self.sc_bridge.as_ref().unwrap();
            for (idx, w) in &exports {
                unsafe { (b.shim.write_export)(*idx, w.as_ptr(), w.len() as i32) };
            }
            // 2. Block until the SystemC slice ran to its next activity.
            let rc = unsafe { (b.shim.sv_wait_slice)() };
            if rc != 0 {
                link.mark_sc_done();
                return false;
            }
            // 3. Adopt the slice's time as the new cap.
            let cap = unsafe { (b.shim.sv_cap)() };
            link.set_cap(cap);
        }
        // 4. Pull the SystemC outputs into this kernel.
        let mut imports = Vec::new();
        self.sc_bridge
            .as_ref()
            .unwrap()
            .collect_imports(&mut imports);
        if let Some(b) = self.sc_bridge.as_mut() {
            b.update_mirrors();
        }
        let had_imports = !imports.is_empty();
        self.sc_apply_imports(imports);
        if had_imports {
            // The import writes bypass the delta machinery (this runs at the
            // event-loop top, outside any time slot): propagate combinational
            // fanout and re-check parked @/always-edge waiters against the
            // new values, mirroring what a $deposit inside a time slot does.
            if self.dirty_any {
                self.settle_combinatorial();
            }
            self.check_edges();
            // A posedge/negedge waiter just ran may have scheduled an NBA
            // (e.g. `dout <= din + 1`); commit it here so the latched value
            // is visible to the next exchange. Without this, SC-top register
            // outputs stay stuck at their initial value (SV time does not
            // advance between exchanges, so the normal loop never commits it).
            // Commit every NBA queue — an always_ff `<=` write lands in
            // `nba_fast`/`nba_queue`, not `delayed_nba` — and repeat until the
            // edge blocks settle (a cascade of `<=`n writes can schedule
            // further edges within the same import).
            self.apply_nba();
            if self.dirty_any {
                self.settle_combinatorial();
            }
        }
        // Publish to the SC side whether any export the SystemC side hasn't
        // yet consumed changed value this round (a combinational fanout that
        // settled only after we staged the exports at the top of this
        // rendezvous). The SC-side drain-to-quiescence loop reads this to
        // decide whether another flush slice is needed before sc_start()
        // returns. Mirrors the pending-export clause of sc_exchange_alive().
        let mut pending = false;
        if let Some(b) = self.sc_bridge.as_ref() {
            for (idx, prev) in &self.sc_last_exports {
                let changed = b.rows.iter().any(|row| {
                    if row.sv_id == *idx
                        && row.dir != crate::sc_bridge::ScDir::Import
                        && row.sc_idx.is_some()
                    {
                        let (v, _x) = self.signal_table[*idx].bits128();
                        let n = row.nwords();
                        let mut cur = vec![0u64; n];
                        cur[0] = v as u64;
                        if n > 1 {
                            cur[1] = (v >> 64) as u64;
                        }
                        cur != *prev
                    } else {
                        false
                    }
                });
                if changed {
                    pending = true;
                    break;
                }
            }
        }
        let b = self.sc_bridge.as_ref().unwrap();
        unsafe { (b.shim.sv_set_pending)(pending as i32) };
        true
    }

}
