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
pub(crate) fn sc_param_value_text(v: &xezim_core::Value) -> String {
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

    fn sc_flat_path(e: &crate::ast::expr::Expression) -> Option<String> {
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

    fn sc_build_net_of(&self, scnames: &std::collections::HashSet<String>) -> std::collections::HashMap<String, String> {
        let mut net_of: std::collections::HashMap<String, String> = Default::default();
        for (key, actual) in &self.module.port_aliases {
            net_of.insert(key.clone(), actual.clone());
        }
        for port in &self.module.input_port_actuals {
            if let Some(a) = Self::sc_flat_path(port.actual()) {
                net_of.insert(port.formal().to_string(), a);
            }
        }
        for ca in &self.module.continuous_assigns {
            if let (Some(lhs), Some(rhs)) = (Self::sc_flat_path(&ca.lhs), Self::sc_flat_path(&ca.rhs)) {
                if let Some(idx) = lhs.rfind('.') {
                    let por = &lhs[idx + 1..];
                    if scnames.contains(por) && !por.contains('.') {
                        net_of.entry(lhs.clone()).or_insert_with(|| rhs.clone());
                    }
                }
            }
        }
        net_of
    }

    /// Chase multi-hop `port_aliases` / `input_port_actuals` chains to the
    /// backing flat signal id in `signal_name_to_id`.
    fn sc_resolve_net(
        &self,
        net_of: &std::collections::HashMap<String, String>,
        start: &str,
    ) -> Option<usize> {
        let mut cur = start;
        for _ in 0..64 {
            if let Some(&id) = self.signal_name_to_id.get(cur) {
                if !net_of.contains_key(cur) {
                    return Some(id);
                }
                // Even if present in both, prefer the downstream root alias if
                // it resolves to a valid signal.
                if let Some(next) = net_of.get(cur) {
                    if next != cur {
                        cur = next.as_str();
                        continue;
                    }
                }
                return Some(id);
            }
            let next = net_of.get(cur)?;
            if next == cur {
                break;
            }
            cur = next.as_str();
        }
        self.signal_name_to_id.get(cur).copied()
    }

    /// Discover all netlist SC-leaf `(instance_path, module_name)` pairs at
    /// arbitrary hierarchy depth using `self.module.instances` (with a flat
    /// fallback for single-level synthetic tests).
    pub(crate) fn sc_leaf_instances(&self) -> Vec<(String, String)> {
        use crate::sc_bridge::sc_module_ports_in;
        let Some(opts) = crate::sc_bridge::sc_options() else {
            return Vec::new();
        };
        let sc_mods = opts.all_sc_modules();
        if sc_mods.is_empty() {
            return Vec::new();
        }
        let sc_set: std::collections::HashSet<&str> =
            sc_mods.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for inst in &self.module.instances {
            if sc_set.contains(inst.def_name.as_str())
                && !sc_module_ports_in(&inst.def_name, &opts.sources, &opts.include_dirs).is_empty()
                && seen.insert(inst.path.clone())
            {
                out.push((inst.path.clone(), inst.def_name.clone()));
            }
        }
        if !out.is_empty() {
            return out;
        }
        // Fallback: single-level net-prefix discovery when `instances` is absent.
        let Some(mname) = opts.sc_module.clone() else {
            return Vec::new();
        };
        let ports = sc_module_ports_in(&mname, &opts.sources, &opts.include_dirs);
        if ports.is_empty() {
            return Vec::new();
        }
        let scnames: std::collections::HashSet<String> =
            ports.iter().map(|p| p.name.clone()).collect();
        let net_of = self.sc_build_net_of(&scnames);
        let mut prefixes: std::collections::BTreeSet<String> = Default::default();
        for key in net_of
            .keys()
            .map(String::as_str)
            .chain(self.signal_name_to_id.keys().map(|k| k.as_ref()))
        {
            if let Some(idx) = key.rfind('.') {
                let (pre, por) = key.split_at(idx);
                let por = &por[1..];
                if scnames.contains(por) && !pre.contains('.') && !pre.is_empty() {
                    prefixes.insert(pre.to_string());
                }
            }
        }
        prefixes.into_iter().map(|p| (p, mname.clone())).collect()
    }

    /// Bind each pure-SystemC leaf instance's port signals at arbitrary SV
    /// hierarchy depth (`u.dout`, `sub.u.dout`, `__xez_isl_0.u.dout`, ...)
    /// into the co-simulation registry.
    pub(crate) fn sc_attach_instance_ports(&mut self, b: &mut crate::sc_bridge::ScBridge) {
        use crate::sc_bridge::{sc_module_ports_in, ScDir, ScPort, ScTop};
        let Some(opts) = crate::sc_bridge::sc_options() else {
            return;
        };
        let leaves = self.sc_leaf_instances();
        if leaves.is_empty() {
            return;
        }
        let mut port_cache: std::collections::HashMap<String, Vec<ScPort>> = Default::default();
        let mut scnames: std::collections::HashSet<String> = Default::default();
        for (_, mname) in &leaves {
            let ports = port_cache
                .entry(mname.clone())
                .or_insert_with(|| sc_module_ports_in(mname, &opts.sources, &opts.include_dirs));
            for p in ports.iter() {
                scnames.insert(p.name.clone());
            }
        }
        let net_of = self.sc_build_net_of(&scnames);
        self.sync_table_to_hashmap();
        let mapped: std::collections::HashSet<String> =
            b.rows.iter().map(|r| r.sc_name.clone()).collect();
        for (inst, mname) in &leaves {
            let Some(ports) = port_cache.get(mname) else {
                continue;
            };
            let mut all_ok = true;
            let mut bindings: Vec<(usize, &ScPort)> = Vec::new();
            for p in ports {
                let sv_name = format!("{}.{}", inst, p.name);
                let Some(id) = self.sc_resolve_net(&net_of, &sv_name) else {
                    all_ok = false;
                    break;
                };
                bindings.push((id, p));
            }
            if !all_ok {
                continue;
            }
            if opts.master == ScTop::Sv {
                self.sc_mode_b = true;
                self.sc_last_exchange_time = 0;
            }
            for (id, p) in bindings {
                let reg_key = format!("{}.{}", inst, p.name);
                if mapped.contains(&reg_key) {
                    continue;
                }
                // Complement mapping: SC sc_in → Export (SV→SC), sc_out →
                // Import (SC→SV), sc_inout → Auto.
                let dir = match p.dir {
                    ScDir::Import => ScDir::Export,
                    ScDir::Export => ScDir::Import,
                    ScDir::Auto => ScDir::Auto,
                };
                b.add_map(id, p.width, reg_key, dir);
            }
        }
    }

    /// Attach ports of multi-hop embedded SV islands instantiated inside C++
    /// (`xez_<SvMod>.h`). Each island's ports are materialized in the SV top as
    /// `<wire_prefix>_<port_ident>` and bound in the C++ registry under
    /// `<sc_inst_name>.<port_name>`.
    pub(crate) fn sc_attach_island_ports(&mut self, b: &mut crate::sc_bridge::ScBridge) {
        use crate::sc_bridge::{ScDir, ScPortDir, ScTop};
        if self.sc_islands.is_empty() {
            return;
        }
        self.sync_table_to_hashmap();
        let islands = self.sc_islands.clone();
        let is_sv_master = crate::sc_bridge::sc_options()
            .is_some_and(|o| o.master == ScTop::Sv);
        let mapped: std::collections::HashSet<String> =
            b.rows.iter().map(|r| r.sc_name.clone()).collect();
        for (wire_pfx, sc_inst, _sv_mod, ports) in &islands {
            for p in ports {
                let sv_sig = format!("{wire_pfx}_{}", p.ident());
                let Some(&sv_id) = self.signal_name_to_id.get(sv_sig.as_str()) else {
                    continue;
                };
                let sc_name = format!("{sc_inst}.{}", p.name);
                if mapped.contains(&sc_name) {
                    continue;
                }
                let dir = match p.dir {
                    ScPortDir::In => ScDir::Import,   // SC drives SV island input
                    ScPortDir::Out => ScDir::Export,  // SV island output drives SC
                    ScPortDir::Inout => ScDir::Auto,
                };
                b.add_map(sv_id, p.width.max(1), sc_name, dir);
                if is_sv_master {
                    self.sc_mode_b = true;
                    self.sc_last_exchange_time = 0;
                }
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
            if name.contains('.') {
                continue;
            }
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

    /// Stage export-mapped 4-state SV signals into the shim, returning
    /// `(sc_idx, sv_id, val_words, xz_words)` tuples and updating `sc_last_exports`.
    fn sc_stage_exports(&mut self) -> Vec<(i32, usize, Vec<u64>, Vec<u64>)> {
        let Some(b) = self.sc_bridge.as_mut() else {
            return Vec::new();
        };
        // $sc_export_sig mappings created via exec_sc_map_task carry a lazy
        // sc_idx; resolve any that are still pending before the first exchange.
        b.ensure_ready();
        let mut out = Vec::new();
        let mut snap = Vec::new();
        for row in &b.rows {
            // Export and Auto rows both push the SV value into SystemC.
            if row.dir == crate::sc_bridge::ScDir::Import {
                continue;
            }
            if let Some(idx) = row.sc_idx {
                let (val, xz) = crate::sc_bridge::value_to_planes(
                    &self.signal_table[row.sv_id],
                    row.sv_width,
                );
                snap.push((row.sv_id, val.clone(), xz.clone()));
                out.push((idx, row.sv_id, val, xz));
            }
        }
        self.sc_last_exports = snap;
        out
    }

    /// True if any Export/Auto row currently differs from the snapshot staged
    /// at the most recent exchange (`sc_last_exports`).
    pub(crate) fn sc_any_export_changed(&self) -> bool {
        let Some(b) = self.sc_bridge.as_ref() else {
            return false;
        };
        for (sv_id, prev_v, prev_x) in &self.sc_last_exports {
            for row in &b.rows {
                if row.sv_id == *sv_id
                    && row.dir != crate::sc_bridge::ScDir::Import
                    && row.sc_idx.is_some()
                {
                    let (cur_v, cur_x) = crate::sc_bridge::value_to_planes(
                        &self.signal_table[*sv_id],
                        row.sv_width,
                    );
                    if cur_v != *prev_v || cur_x != *prev_x {
                        return true;
                    }
                    break;
                }
            }
        }
        false
    }

    /// Apply pulled 4-state import values into the signal table with full write
    /// bookkeeping (`write_sig!` + dirty tracking + `table_modified`). Returns
    /// true when at least one SV signal actually changed.
    fn sc_apply_imports(&mut self, imports: Vec<(usize, Vec<u64>, Vec<u64>)>) -> bool {
        let mut any_changed = false;
        for (sv_id, val_words, xz_words) in imports {
            let w_bits = self.signal_table[sv_id].width;
            let mut val = crate::sc_bridge::planes_to_value(&val_words, &xz_words, w_bits);
            if self.signal_two_state.get(sv_id).copied().unwrap_or(false) {
                val = val.to_two_state();
            }
            if self.signal_table[sv_id] != val {
                any_changed = true;
                write_sig!(self, sv_id, val);
                self.after_signal_write(sv_id);
                if sv_id < self.dirty_signals.len() && !self.dirty_signals[sv_id] {
                    self.dirty_signals[sv_id] = true;
                    self.dirty_list.push(sv_id);
                    self.dirty_any = true;
                }
                self.table_modified = true;
            }
        }
        self.sc_last_import_changed = any_changed;
        any_changed
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
            for (idx, _sv_id, val, xz) in &exports {
                b.shim.write_export_planes(*idx, val, xz);
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
            for (idx, _sv_id, val, xz) in &exports {
                b.shim.write_export_planes(*idx, val, xz);
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
    /// SystemC side hasn't consumed yet.
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
        self.sc_any_export_changed()
    }

    pub(crate) fn sc_rendezvous(&mut self) -> bool {
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
        // 1. Publish this kernel's exports for the slice.
        {
            let b = self.sc_bridge.as_ref().unwrap();
            for (idx, _sv_id, val, xz) in &exports {
                b.shim.write_export_planes(*idx, val, xz);
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
        let changed = self.sc_apply_imports(imports);
        if changed {
            if self.dirty_any {
                self.settle_combinatorial();
            }
            self.check_edges();
            self.apply_nba();
            if self.dirty_any {
                self.settle_combinatorial();
            }
        }
        let pending = self.sc_any_export_changed();
        let b = self.sc_bridge.as_ref().unwrap();
        unsafe { (b.shim.sv_set_pending)(pending as i32) };
        true
    }

}
