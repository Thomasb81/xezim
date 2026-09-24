//! IEEE 1800-2017 §30.4/§30.5 module path delays.
//!
//! Each path-delayed net carries its module paths
//! (`ElaboratedModule::module_paths`): twelve transition delays, the input
//! terminals, and an optional `if` condition or `ifnone`. A driver of such a
//! net reaches `schedule_delayed_with_delay`, which picks the delay here:
//!
//! * a path is enabled when it is unconditional, its condition is true or
//!   x/z (§30.4.4.1), or — `ifnone` — no conditional path from one of its
//!   inputs is enabled; with no enabled path the net changes at once;
//! * among the enabled paths, the one whose input changed most recently
//!   wins; inputs that changed together take the smallest delay (§30.4.4);
//! * each bit uses the delay of its own transition (§30.5.1 Table 30-3),
//!   so the bits of a bus that rise and fall together settle separately.
//!
//! Input change times come from `TcWatch` comb entries (the timing-check
//! watches), which run after the combinational logic of a settle — a
//! driver may schedule before the watch of the input that triggered it has
//! run, so an input whose value still differs from its watch's last-seen
//! value counts as changing now.
//!
//! Not modelled: the edge of an edge-sensitive path (the path applies on
//! any change of its input) and PATHPULSE$ limits — the reference simulator
//! also rejects every pulse narrower than the path delay by default.
use super::*;

/// A module path into one net.
pub(super) struct PathRt {
    delays: [u64; 12],
    /// Indices into `Simulator::path_srcs`.
    srcs: Vec<u32>,
    cond: Option<Arc<Expression>>,
    ifnone: bool,
}

/// A path input terminal: its signal, the watch stamping its changes, and
/// the time of its last change.
pub(super) struct PathSrc {
    sig: usize,
    watch: usize,
    pub(super) last: u64,
}

impl Simulator {
    /// Resolve `ElaboratedModule::module_paths` to signal ids. Nets whose
    /// delay SDF back-annotated keep the SDF value (it replaces the path
    /// delays), so they get no path table.
    pub(super) fn build_module_paths(
        &mut self,
        skip: &std::collections::HashSet<usize>,
        map_delay: impl Fn(u64) -> u64,
    ) {
        let paths = std::mem::take(&mut self.module.module_paths);
        let mut names: Vec<&String> = paths.keys().collect();
        names.sort();
        let mut src_of: HashMap<usize, u32> = HashMap::default();
        for name in names {
            let Some(&id) = self.signal_name_to_id.get(name.as_str()) else {
                continue;
            };
            if skip.contains(&id) {
                continue;
            }
            let mut rts = Vec::new();
            for p in &paths[name] {
                let mut srcs = Vec::new();
                for s in &p.srcs {
                    let Some(&sig) = self.signal_name_to_id.get(s.as_str()) else {
                        continue;
                    };
                    let slot = match src_of.get(&sig) {
                        Some(&k) => k,
                        None => {
                            let k = self.path_srcs.len() as u32;
                            let watch = self.path_source_watch(sig, k);
                            self.path_srcs.push(PathSrc {
                                sig,
                                watch,
                                last: 0,
                            });
                            src_of.insert(sig, k);
                            k
                        }
                    };
                    srcs.push(slot);
                }
                rts.push(PathRt {
                    delays: p.delays.map(&map_delay),
                    srcs,
                    cond: p.cond.clone().map(Arc::new),
                    ifnone: p.ifnone,
                });
            }
            self.path_dst.insert(id, self.module_paths.len());
            self.module_paths.push(rts);
        }
    }

    /// Runtime SDF back-annotation replaces the path delays of the nets it
    /// annotates.
    pub(super) fn drop_module_paths_of(&mut self, id: usize) {
        self.path_dst.remove(&id);
    }

    /// When the path's inputs last changed.
    fn path_input_time(&self, srcs: &[u32]) -> u64 {
        srcs.iter()
            .map(|&s| {
                let src = &self.path_srcs[s as usize];
                if self.timing_watch_stale(src.watch, src.sig) {
                    self.time
                } else {
                    src.last
                }
            })
            .max()
            .unwrap_or(0)
    }

    fn path_cond_enabled(&mut self, c: &Arc<Expression>) -> bool {
        // The condition is already in the flat namespace; a scope hint left
        // behind by the previous evaluation must not re-root its names.
        let saved = self.name_resolve_hint.borrow_mut().take();
        let v = self.eval_expr(c);
        *self.name_resolve_hint.borrow_mut() = saved;
        v.has_xz() || v.is_true()
    }

    /// The per-transition delays that apply to the next change of the net
    /// with path table `k`; `None` when no path is enabled.
    fn module_path_delays(&mut self, k: usize) -> Option<[u64; 12]> {
        let n = self.module_paths[k].len();
        let mut enabled = vec![false; n];
        for i in 0..n {
            let (cond, ifnone) = {
                let p = &self.module_paths[k][i];
                (p.cond.clone(), p.ifnone)
            };
            enabled[i] = match cond {
                _ if ifnone => false,
                Some(c) => self.path_cond_enabled(&c),
                None => true,
            };
        }
        let paths = &self.module_paths[k];
        for i in 0..n {
            if paths[i].ifnone {
                enabled[i] = !(0..n).any(|j| {
                    enabled[j]
                        && paths[j].cond.is_some()
                        && paths[j].srcs.iter().any(|s| paths[i].srcs.contains(s))
                });
            }
        }
        let times: Vec<u64> = (0..n)
            .map(|i| {
                if enabled[i] {
                    self.path_input_time(&paths[i].srcs)
                } else {
                    0
                }
            })
            .collect();
        let latest = (0..n).filter(|&i| enabled[i]).map(|i| times[i]).max()?;
        let mut out = [u64::MAX; 12];
        for i in (0..n).filter(|&i| enabled[i] && times[i] == latest) {
            for (o, d) in out.iter_mut().zip(paths[i].delays) {
                *o = (*o).min(d);
            }
        }
        Some(out)
    }

    /// Schedule `val` onto the path-delayed net `id` (path table `k`): each
    /// bit after the delay of its own transition. Inertial per bit: a bit
    /// already on its way to the same value keeps its pending time, any other
    /// pending change of the net is replaced, and a bit with no delay
    /// commits at once.
    pub(super) fn schedule_module_path(&mut self, id: usize, k: usize, val: Value) {
        let delays = self.module_path_delays(k);
        let old = self.signal_table[id].clone();
        // The net's queued updates, oldest first; each carries every bit
        // changed by the ones before it.
        let mut pending: Vec<(u64, Value)> = self
            .delayed_updates
            .iter()
            .filter(|(_, sid, _)| *sid == id)
            .map(|(t, _, v)| (*t, v.clone()))
            .collect();
        pending.sort_by_key(|(t, _)| *t);
        self.delayed_updates.retain(|(_, sid, _)| *sid != id);
        let width = (old.width as usize).min(val.width as usize);
        // (when, bit) for every bit that changes.
        let mut when: Vec<(u64, usize)> = Vec::new();
        for b in 0..width {
            let (from, to) = (old.get_bit_code(b), val.get_bit_code(b));
            let Some(t) = xezim_core::elaborate::path_transition_index(from, to) else {
                continue;
            };
            let in_flight = pending
                .last()
                .filter(|(_, v)| v.get_bit_code(b) == to)
                .and_then(|_| pending.iter().find(|(_, v)| v.get_bit_code(b) == to))
                .map(|(t, _)| *t);
            let at = in_flight.unwrap_or_else(|| self.time + delays.map_or(0, |ds| ds[t]));
            when.push((at, b));
        }
        when.sort_unstable();
        let mut cur = old;
        let mut i = 0;
        while i < when.len() {
            let at = when[i].0;
            while i < when.len() && when[i].0 == at {
                let b = when[i].1;
                cur.set_bit_code(b, val.get_bit_code(b));
                i += 1;
            }
            let v = if i == when.len() {
                val.clone()
            } else {
                cur.clone()
            };
            if at <= self.time {
                self.commit_delayed_now(id, v);
            } else {
                self.delayed_updates.push((at, id, v));
            }
        }
    }

    /// Commit a delayed update whose delay came out as zero, as
    /// `apply_delayed_updates` does when one matures.
    fn commit_delayed_now(&mut self, id: usize, mut val: Value) {
        val.is_signed = self.signal_signed[id];
        if self.signal_table[id] != val {
            write_sig!(self, id, val);
            self.mark_dirty_id(id);
            self.table_modified = true;
        }
    }
}
