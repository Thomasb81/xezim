//! Class-object reclamation (IEEE 1800-2017 §8.7: an object no handle reaches
//! may be reclaimed). Mark from a conservative trace of every scheduler-owned
//! root, follow typed object edges, then sweep each unmarked object together
//! with its owned storage. Collection runs only at a time advance of the outer
//! event loop, where no Rust frame holds values outside the traced state.
use super::heap_trace::{HandleTrace, TraceError};
use super::*;

/// Fewest allocations between two collections.
const CLASS_GC_MIN_ALLOCS: usize = 20_000;

/// Fewest name-keyed values (`signals`) added between two collections. The
/// elements of an object's fixed array are stored there, one entry each, so a
/// few objects can own most of the store: 16,384 objects with a 256-element
/// array stay under `CLASS_GC_MIN_ALLOCS` while holding four million entries.
const CLASS_GC_MIN_SIGNALS: usize = 100_000;

/// `XEZIM_CLASS_GC=0` disables reclamation; `XEZIM_CLASS_GC=stress` collects
/// at every time advance that allocated an object (for validating roots).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ClassGcMode {
    Off,
    On,
    Stress,
}

pub(super) fn class_gc_mode() -> ClassGcMode {
    static MODE: std::sync::OnceLock<ClassGcMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("XEZIM_CLASS_GC").as_deref() {
        Ok("0") | Ok("off") => ClassGcMode::Off,
        Ok("stress") => ClassGcMode::Stress,
        _ => ClassGcMode::On,
    })
}

/// Runtime namespaces owned by a class slot or a private collection
/// declaration. A fork capture keeps the declaration's namespace alive.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum StorageOwner {
    Class(usize),
    Local(u64),
}

impl StorageOwner {
    #[inline]
    pub(super) fn of(name: &str) -> Option<Self> {
        if let Some(local) = name.strip_prefix('@') {
            // The declaration suffix is #id. Any element or member names
            // follow it, including arbitrary associative string keys.
            let head = local.split_once('[').map_or(local, |(head, _)| head);
            head.rmatch_indices('#').find_map(|(at, _)| {
                let id = head[at + 1..].split('.').next()?;
                if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                id.parse().ok().map(Self::Local)
            })
        } else {
            Simulator::class_storage_owner(name).map(Self::Class)
        }
    }
}

/// Reverse index for name-keyed storage owned by class objects and
/// private collection declarations.
#[derive(Default)]
pub(super) struct ClassStorageKeys(HashMap<StorageOwner, HashSet<String>>);

impl ClassStorageKeys {
    pub(super) fn insert(&mut self, name: &str) {
        if let Some(owner) = StorageOwner::of(name) {
            self.0.entry(owner).or_default().insert(name.to_owned());
        }
    }

    pub(super) fn remove(&mut self, name: &str) {
        if let Some(owner) = StorageOwner::of(name)
            && let std::collections::hash_map::Entry::Occupied(mut entry) = self.0.entry(owner)
        {
            entry.get_mut().remove(name);
            if entry.get().is_empty() {
                entry.remove();
            }
        }
    }

    pub(super) fn names(&self, owner: StorageOwner) -> impl Iterator<Item = &str> {
        self.0.get(&owner).into_iter().flatten().map(String::as_str)
    }

    pub(super) fn take_matching(
        &mut self,
        owner: StorageOwner,
        discard: &impl Fn(&str) -> bool,
    ) -> Vec<String> {
        let std::collections::hash_map::Entry::Occupied(mut entry) = self.0.entry(owner) else {
            return Vec::new();
        };
        let removed = entry.get_mut().extract_if(|name| discard(name)).collect();
        if entry.get().is_empty() {
            entry.remove();
        }
        removed
    }

    pub(super) fn take(&mut self, owner: StorageOwner) -> Option<HashSet<String>> {
        self.0.remove(&owner)
    }
}

/// A table keyed by a class slot (handle and property or constraint name).
/// Keep the flat table for ordinary lookup and checkpoint compatibility.
#[derive(serde::Serialize)]
#[serde(transparent)]
pub(super) struct ClassSlotMap<V> {
    slots: HashMap<(usize, String), V>,
    #[serde(skip)]
    names: HashMap<usize, HashSet<String>>,
}

impl<V> Default for ClassSlotMap<V> {
    fn default() -> Self {
        Self {
            slots: HashMap::default(),
            names: HashMap::default(),
        }
    }
}

impl<'de, V: serde::Deserialize<'de>> serde::Deserialize<'de> for ClassSlotMap<V> {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        let slots: HashMap<(usize, String), V> = serde::Deserialize::deserialize(decoder)?;
        let mut indexed = Self {
            slots,
            ..Self::default()
        };
        for (owner, name) in indexed.slots.keys() {
            indexed
                .names
                .entry(*owner)
                .or_default()
                .insert(name.clone());
        }
        Ok(indexed)
    }
}

impl<V> std::ops::Deref for ClassSlotMap<V> {
    type Target = HashMap<(usize, String), V>;

    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl<V> ClassSlotMap<V> {
    pub(super) fn entry(
        &mut self,
        key: (usize, String),
    ) -> std::collections::hash_map::Entry<'_, (usize, String), V> {
        let entry = self.slots.entry(key);
        if let std::collections::hash_map::Entry::Vacant(vacant) = &entry {
            let (owner, name) = vacant.key();
            self.names.entry(*owner).or_default().insert(name.clone());
        }
        entry
    }

    pub(super) fn insert(&mut self, key: (usize, String), value: V) -> Option<V> {
        match self.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => Some(entry.insert(value)),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(value);
                None
            }
        }
    }

    pub(super) fn get_mut(&mut self, key: &(usize, String)) -> Option<&mut V> {
        self.slots.get_mut(key)
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = (&(usize, String), &mut V)> {
        self.slots.iter_mut()
    }

    pub(super) fn remove(&mut self, key: &(usize, String)) -> Option<V> {
        let value = self.slots.remove(key);
        let (owner, name) = key;
        if let std::collections::hash_map::Entry::Occupied(mut entry) = self.names.entry(*owner) {
            entry.get_mut().remove(name);
            if entry.get().is_empty() {
                entry.remove();
            }
        }
        value
    }

    pub(super) fn clear(&mut self) {
        self.slots.clear();
        self.names.clear();
    }

    pub(super) fn remove_owner(
        &mut self,
        owner: usize,
        mut removed: impl FnMut((usize, String), V),
    ) {
        if let Some(names) = self.names.remove(&owner) {
            for name in names {
                if let Some((slot, value)) = self.slots.remove_entry(&(owner, name)) {
                    removed(slot, value);
                }
            }
        }
    }
}

/// The metadata tables that accept owned collection names. This list
/// drives both direct teardown lookups and reconstruction on restore.
macro_rules! class_metadata_fields {
    ($visit:ident) => {
        $visit! {
            ModuleArrays => module.arrays; keys,
            ModuleArrays2d => module.arrays_2d; keys,
            ModuleArraysNd => module.arrays_nd; keys,
            ModuleAssociativeArrays => module.associative_arrays; keys,
            ModuleAssocElemWidths => module.assoc_elem_widths; keys,
            ModuleAssocIndexWidths => module.assoc_index_widths; keys,
            ModuleAssocKeyTypeNames => module.assoc_key_type_names; keys,
            ModuleAssocDefaults => module.assoc_defaults; keys,
            ModuleQueueMaxSizes => module.queue_max_sizes; keys,
            ModuleVarDeclTypes => module.var_decl_types; keys,
            ModulePackedFullDims => module.packed_full_dims; keys,
            ModulePackedStructFields => module.packed_struct_fields; keys,
            ModulePackedSignalElemWidths => module.packed_signal_elem_widths; keys,
            ModuleCollShapes => module.coll_shapes; keys,
            ModuleArrayElemClass => module.array_elem_class; keys,
            ModuleUnpackedDeclDims => module.unpacked_decl_dims; keys,
            ModuleAscendingPacked => module.ascending_packed; keys,
            ModuleStructMembers => module.struct_members; keys,
            ModuleClassTypeArgs => module.class_type_args; keys,
            ModuleDynamicArrays => module.dynamic_arrays; iter,
            ModuleDescendingArrays => module.descending_arrays; iter,
            ModuleQueueVars => module.queue_vars; iter,
            ModuleStringSignals => module.string_signals; iter,
            ModuleTwoStateSignals => module.two_state_signals; iter,
            VarClassTypes => var_class_types; keys,
            VarTypedefTypes => var_typedef_types; keys,
            VarContainerTypes => var_container_types; keys,
            VarTypeArgs => var_type_args; keys,
            ClassAssocDefaults => class_assoc_defaults; keys,
            DynamicAssocDims => dynamic_assoc_dims; keys,
            LocalCollShapes => local_coll_shapes; keys,
            LocalCollElems => local_coll_elems; keys,
            ActiveUnionTag => active_union_tag; keys,
            RandRanges => rand_ranges; keys,
            RandElemRanges => rand_elem_ranges; keys,
            StaticStructHolders => static_struct_holders; keys,
            SignedSignals => signed_signals; iter,
            StringSignals => string_signals; iter,
            RealSignals => real_signals; iter,
            MultiDimArrayNames => multi_dim_array_names; iter,
            ArrayLocalPackedDims => array_local_packed_dims; iter,
            AssocElemColls => assoc_elem_colls; iter,
            SignedClassColls => signed_class_colls; iter,
            MixedPackedVars => mixed_packed_vars; iter,
            StaticStructNames => static_struct_names; iter,
        }
    };
}

macro_rules! define_metadata_fields {
    ($($kind:ident => $($field:ident).+; $iter:ident,)*) => {
        #[derive(Clone, Copy)]
        pub(super) enum ClassMetadataField {
            $($kind,)*
        }
    };
}

class_metadata_fields!(define_metadata_fields);

impl ClassMetadataField {
    #[allow(non_upper_case_globals)]
    pub(super) const fn named(path: &str) -> Self {
        macro_rules! visit {
            ($($kind:ident => $first:ident$(.$field:ident)*; $iter:ident,)*) => {{
                $(const $kind: &[u8] = concat!(
                    stringify!($first) $(, ".", stringify!($field))*
                ).as_bytes();)*
                match path.as_bytes() {
                    $($kind => Self::$kind,)*
                    _ => panic!("class metadata field is not registered"),
                }
            }};
        }
        class_metadata_fields!(visit)
    }

    fn bit(self) -> u64 {
        1u64 << self as u64
    }
}

#[derive(Default)]
pub(super) struct ClassMetadataKeys {
    /// Each per-owner name has one bit for each table that holds it.
    owners: HashMap<StorageOwner, HashMap<String, u64>>,
}

impl ClassMetadataKeys {
    pub(super) fn record<K: AsRef<str>>(&mut self, field: ClassMetadataField, key: K) -> K {
        let name = key.as_ref();
        if let Some(owner) = StorageOwner::of(name) {
            let names = self.owners.entry(owner).or_default();
            if let Some(bits) = names.get_mut(name) {
                *bits |= field.bit();
            } else {
                names.insert(name.to_owned(), field.bit());
            }
        }
        key
    }

    pub(super) fn forget<K: AsRef<str>>(&mut self, field: ClassMetadataField, key: K) -> K {
        let name = key.as_ref();
        if let Some(owner) = StorageOwner::of(name)
            && let std::collections::hash_map::Entry::Occupied(mut entry) = self.owners.entry(owner)
        {
            let names = entry.get_mut();
            if let Some(bits) = names.get_mut(name) {
                *bits &= !field.bit();
                if *bits == 0 {
                    names.remove(name);
                }
            }
            if names.is_empty() {
                entry.remove();
            }
        }
        key
    }
}

impl Simulator {
    /// The index is derived from the restored tables, so old checkpoints
    /// keep the same representation.
    pub(super) fn rebuild_class_metadata_keys(&mut self) {
        self.class_metadata_keys = self.collect_class_metadata_keys();
    }

    #[cfg(test)]
    pub(super) fn class_metadata_keys_are_current(&self) -> bool {
        self.class_metadata_keys.owners == self.collect_class_metadata_keys().owners
    }

    pub(super) fn release_owned_metadata(&mut self, owner: StorageOwner) {
        if let Some(names) = self.class_metadata_keys.owners.remove(&owner) {
            self.remove_indexed_metadata(names);
        }
    }

    fn remove_indexed_metadata(&mut self, names: impl IntoIterator<Item = (String, u64)>) {
        for (name, bits) in names {
            macro_rules! remove {
                ($($kind:ident => $first:ident$(.$field:ident)*; $iter:ident,)*) => {
                    $(if bits & ClassMetadataField::$kind.bit() != 0 {
                        self.$first$(.$field)*.remove(name.as_str());
                    })*
                };
            }
            class_metadata_fields!(remove);
        }
    }

    /// Drop every runtime registration underneath a deleted associative row.
    pub(super) fn release_owned_collection_rows(
        &mut self,
        owner: StorageOwner,
        discard: impl Fn(&str) -> bool,
    ) {
        self.signals.release_matching_storage(owner, &discard);
        self.widths.release_matching_storage(owner, &discard);
        let Some(names) = self.class_metadata_keys.owners.get_mut(&owner) else {
            return;
        };
        let removed: Vec<_> = names.extract_if(|name, _| discard(name)).collect();
        if names.is_empty() {
            self.class_metadata_keys.owners.remove(&owner);
        }
        self.remove_indexed_metadata(removed);
    }

    pub(super) fn replace_rand_ranges(&mut self, ranges: HashMap<String, Vec<(i128, i128)>>) {
        for name in self.rand_ranges.keys() {
            self.class_metadata_keys
                .forget(ClassMetadataField::RandRanges, name);
        }
        for name in ranges.keys() {
            self.class_metadata_keys
                .record(ClassMetadataField::RandRanges, name);
        }
        self.rand_ranges = ranges;
    }

    fn collect_class_metadata_keys(&self) -> ClassMetadataKeys {
        let mut index = ClassMetadataKeys::default();
        macro_rules! visit {
            ($($kind:ident => $first:ident$(.$field:ident)*; $iter:ident,)*) => {
                $(for name in self.$first$(.$field)*.$iter() {
                    index.record(ClassMetadataField::$kind, name);
                })*
            };
        }
        class_metadata_fields!(visit);
        index
    }
}

impl Simulator {
    /// Collect unreachable class objects once enough have been allocated
    /// since the last collection. Called between time slots.
    pub(super) fn maybe_collect_class_heap(&mut self) {
        let mode = class_gc_mode();
        if mode == ClassGcMode::Off {
            return;
        }
        let grown = self.heap.len().saturating_sub(self.class_gc_heap_len);
        let grown_signals = self
            .signals
            .len()
            .saturating_sub(self.class_gc_live_signals);
        // A collection traces the previous survivors and the slots allocated
        // since, not every slot ever allocated, so its cost follows the live
        // objects plus the new ones and a fixed threshold stays linear over a
        // long run.
        let (threshold, signal_threshold) = match mode {
            // Stress keeps to its definition: a time advance that allocated.
            ClassGcMode::Stress => (1, usize::MAX),
            _ => (
                CLASS_GC_MIN_ALLOCS.max(self.class_gc_live),
                CLASS_GC_MIN_SIGNALS.max(self.class_gc_live_signals),
            ),
        };
        // A suspended DPI task keeps values on a foreign stack the trace
        // cannot see; wait until none is parked.
        if (grown < threshold && grown_signals < signal_threshold) || !self.dpi_tasks.is_empty() {
            return;
        }
        self.class_gc_heap_len = self.heap.len();
        // Both counts restart here, before the trace: one that fails (or a
        // design whose store grows without any object) must wait for the next
        // threshold like the allocation count does, not retry the root trace
        // at every time advance.
        self.class_gc_live_signals = self.signals.len();
        let objects: Vec<usize> = self
            .class_gc_survivors
            .iter()
            .copied()
            .chain(self.class_gc_scan_from.max(1)..self.heap.len())
            .filter(|&h| self.heap[h].is_some())
            .collect();
        // No object, nothing to release: skip the trace of every root.
        if objects.is_empty() {
            return;
        }
        let Ok(marked) = self.class_heap_mark() else {
            return;
        };
        self.class_gc_scan_from = self.heap.len();
        let (live, dead_list): (Vec<usize>, Vec<usize>) =
            objects.into_iter().partition(|&h| marked.get(h));
        let released = self.release_class_storage(&dead_list);
        self.class_gc_live = live.len();
        self.class_gc_live_signals = self.signals.len();
        self.class_gc_survivors = live;
        self.class_gc_released += released;
        self.class_gc_runs += 1;
    }
}

impl Simulator {
    /// Trace scheduler-owned runtime roots without interpreting any expressions.
    /// The sweep runs only at a time advance with no suspended DPI task
    /// (`maybe_collect_class_heap`), whose foreign stack the trace cannot see.
    pub(super) fn trace_class_runtime_roots(&self) -> Result<HandleTrace, TraceError> {
        let mut trace = HandleTrace::new(self.heap.len());
        macro_rules! roots {
            ($($field:ident),* $(,)?) => { $(trace.trace(&self.$field)?;)* };
        }
        roots!(
            forced_signals,
            static_fn_ret,
            active_force_exprs,
            partial_forces,
            pending_strobes,
            pending_observed,
            clocking_snapshots,
            clocking_output_pending,
            clocking_cycle_pending,
            clocking_edge_out_pending,
            clocking_preponed,
            clocking_hist,
            deferred_clocking_conts,
            deferred_clocking_guarded,
            pending_reactive,
            reactive_conts,
            static_struct_holders,
            monitor,
            monitor_prev,
            monitor_arg_prev,
            delayed_updates,
            delayed_nba,
            delayed_nba_queue,
            packed_nba,
            this_stack,
            local_stack,
            block_scope_saves,
            return_value,
            queue_frame_saves,
            task_cleanup,
            ref_binding_stack,
            ref_alias_stack,
            local_dyn,
            formal_dyn,
            static_local_syncs,
            decl_shadow_log,
            static_local_vars,
            join_waiters,
            await_waiters,
            vm_regs,
            vm_ret_vals,
            foreach_arena,
            ts_regs,
            ts_wregs,
            ts_wregs8,
            ts_save_list,
            nba_queue,
            pending_nba_instance_triggers,
            event_aliases,
            event_waiters,
            instance_event_waiters,
            triggered_conts_buf,
            condition_waiters,
            ready_condition_waiters,
            inactive_queue,
            nba_region_waiters,
            fork_spawn_cont,
            exec_park_cont,
            slot_batches,
            deferred_comb,
            settle_prev_values,
            process_loop_bytecode,
            process_cond_bytecode,
            forever_cont_cache,
            forever_sens_cache,
            blocking_cont_cache,
            task_frame_eligibility,
            class_task_frame_cache,
            compiled_fast_calls,
            compiled_fast_static_calls,
            pending_struct_coll_wb,
            pending_ret_struct,
            pending_ret_collection,
            pending_vif_return,
            vm_ret_coll,
            pending_formal_dyn,
            static_vif_bindings,
            local_iface_aliases,
            viface_var_aliases,
            frame_pool,
            queue_save_pool,
            ctor_class_stack,
            obj_rng_stack,
            factory_reg_in_progress,
            var_type_args,
            sampled_watches,
            sampled_defaults,
            proc_sampled_blocks,
            sva_lv,
            sva_proc_pending,
            sva_default_disable,
            sva_preponed,
            cg_heap,
            cg_named_event_waiters,
            cg_instance_event_waiters,
            cg_type_options
        );
        // Process IDs, delay slots, signal IDs and cache coordinates
        // are identifiers for their own tables (§8.7). Only the captured
        // data under them participates in class reachability.
        macro_rules! captured_values {
            ($($field:ident),* $(,)?) => {
                $(for value in self.$field.values() {
                    trace.trace(value)?;
                })*
            };
        }
        captured_values!(
            process_contexts,
            fork_baselines,
            fork_signal_captures,
            suspended_proc_info,
            intra_saved,
            method_receiver_cache,
            cond_waiter_conditions,
        );
        for (_, value) in self.vc_saved.values() {
            trace.trace(value)?;
        }
        for (_, _, value) in &self.recv_call_memo {
            trace.trace(value)?;
        }
        for frames in self.suspended_loop_frames.values() {
            for statements in frames.values() {
                trace.trace(statements)?;
            }
        }
        for (id, value) in self.signal_table.iter().enumerate() {
            let previous = self.prev_val.get(id).copied().unwrap_or(0);
            // Narrow values without an in-range handle cannot carry an event
            // receiver. Skip the declaration lookup for the common zero case.
            if value.width < 56
                && !trace.can_candidate(value.raw_bits().0)
                && !trace.can_candidate(previous)
            {
                continue;
            }
            // Use existing names only. Virtual bulk-memory cells must not
            // materialize a name as a side effect of GC.
            if !self
                .id_to_name
                .get(id)
                .is_some_and(|name| self.named_storage_has_no_handles(name))
            {
                trace.trace_value(value)?;
                trace.candidate(previous);
            }
        }
        for (&id, value) in &self.prev_wide {
            if !self
                .id_to_name
                .get(id)
                .is_some_and(|name| self.named_storage_has_no_handles(name))
            {
                trace.trace_value(value)?;
            }
        }
        // task_frame_eligibility retains every declaration used by this cache.
        // Trace each immutable declaration once there, and only the frames
        // here; visiting both maps whole repeats declaration bodies per site.
        for (_, frame) in self.task_frame_cache.values() {
            if let Some(frame) = frame {
                trace.trace(frame)?;
            }
        }
        for (name, value) in self
            .signals
            .map_entries()
            .chain(self.class_assoc_defaults.iter())
        {
            // An object's own storage is an edge from that object.
            if self.live_storage_owner(name).is_none() {
                self.trace_named_storage(&mut trace, name, std::iter::once(value))?;
            }
        }
        for (name, _, values) in self.signals.elem_runs() {
            if self.live_storage_owner(name).is_none() {
                self.trace_named_storage(&mut trace, name, values)?;
            }
        }
        for entry in self.class_statics.iter() {
            trace.trace(&entry)?;
        }
        for list in self
            .event_queue
            .wheel
            .iter()
            .chain(self.event_queue.overflow.values())
        {
            trace.trace(list)?;
        }
        // A blocked operation keeps its mailbox/semaphore alive, including its
        // captured value, output lvalue and continuation. Queued values themselves
        // are edges from the owning mailbox, added during object traversal.
        macro_rules! nonempty_waiter_roots {
            ($($field:ident),* $(,)?) => {
                $(for (&handle, waiters) in &self.$field {
                    if !waiters.is_empty() {
                        trace.candidate(handle as u64);
                        trace.trace(waiters)?;
                    }
                })*
            };
        }
        nonempty_waiter_roots!(
            mailbox_put_waiters,
            mailbox_get_waiters,
            semaphore_get_waiters
        );
        for waiter in self.mailbox_put_waiters.values().flatten() {
            if let Some(items) = self.mbx_pattern_items(&waiter.value) {
                trace.trace(items)?;
            }
        }
        for fsm in self.proc_fsm.values() {
            trace.trace(&fsm.regs)?;
            trace.trace(&fsm.compiled)?;
            trace.trace(&fsm.waits)?;
            if let Some(ts) = &fsm.ts {
                trace.trace(&ts.regs)?;
                trace.trace(&ts.snap)?;
            }
            #[cfg(feature = "jit")]
            trace.trace(&fsm.native_frame)?;
        }
        for entry in self.compiled_method_block_cache.values() {
            if let super::super::bytecode::CompiledMethodOutcome::Block(entry) = entry {
                trace.trace(&entry.block)?;
            }
        }
        for handle in self.pending_class_initializers.keys() {
            trace.candidate(*handle as u64);
        }
        for site in &self.sva_sites {
            trace.trace(&(
                &site.events,
                &site.node,
                &site.disable,
                &site.attempts,
                &site.past_snapshots,
                &site.pass_action,
                &site.fail_action,
                &site.sels,
                &site.lv_init,
                &site.captured,
                &site.matured,
            ))?;
        }
        for report in &self.deferred_asserts {
            trace.trace(&report.this_handle)?;
            trace.trace(&report.action)?;
        }
        Ok(trace)
    }

    #[inline]
    pub(super) fn class_storage_owner(name: &str) -> Option<usize> {
        let mut bytes = name.bytes();
        let first = bytes.next()?.wrapping_sub(b'0');
        if first > 9 {
            return None;
        }
        let mut owner = usize::from(first);
        for byte in bytes {
            match byte {
                b'#' => return Some(owner),
                b'0'..=b'9' => {
                    owner = owner
                        .checked_mul(10)?
                        .checked_add(usize::from(byte - b'0'))?;
                }
                _ => return None,
            }
        }
        None
    }
}

impl Simulator {
    /// The objects reachable from the runtime roots, transitively, marked
    /// over every allocated object rather than the previous survivors.
    #[cfg(test)]
    pub(super) fn class_heap_reachability_probe(&self) -> Result<Vec<bool>, TraceError> {
        let marked = self.class_heap_mark()?;
        Ok(self
            .heap
            .iter()
            .enumerate()
            .map(|(h, object)| object.is_some() && marked.get(h))
            .collect())
    }

    fn live_storage_owner(&self, name: &str) -> Option<usize> {
        Self::class_storage_owner(name)
            .filter(|&owner| self.heap.get(owner).is_some_and(Option::is_some))
    }

    /// Follow edges only from reachable objects (§8.7). The same owner
    /// index used by sweep locates their sparse keys and dense runs.
    fn class_heap_mark(&self) -> Result<super::heap_trace::SlotBits, TraceError> {
        let mut trace = self.trace_class_runtime_roots()?;
        let mut todo = Vec::new();
        trace.append_candidates(&mut todo);
        while let Some(handle) = todo.pop() {
            let Some(object) = self.heap.get(handle).and_then(Option::as_ref) else {
                continue;
            };
            for (property, value) in object.properties.iter() {
                if !self.class_property_has_no_handles(handle, property) {
                    trace.trace_value(value)?;
                }
            }
            if let Some(values) = self.mailboxes.get(&handle) {
                for value in values {
                    if let Some(items) = self.mbx_pattern_items(value) {
                        trace.trace(items)?;
                    } else {
                        trace.trace_value(value)?;
                    }
                }
            }
            let owner = StorageOwner::Class(handle);
            let (entries, runs) = self.signals.owner_storage(owner);
            for (name, value) in entries {
                self.trace_named_storage(&mut trace, name, std::iter::once(value))?;
            }
            for (name, values) in runs {
                self.trace_named_storage(&mut trace, name, values)?;
            }
            if let Some(names) = self.class_metadata_keys.owners.get(&owner) {
                for (name, bits) in names {
                    if bits & ClassMetadataField::ClassAssocDefaults.bit() != 0
                        && let Some(value) = self.class_assoc_defaults.get(name)
                    {
                        self.trace_named_storage(&mut trace, name, std::iter::once(value))?;
                    }
                }
            }
            trace.append_candidates(&mut todo);
        }
        Ok(trace.candidates)
    }

    /// Classify a selector in the context of the collection being traced.
    /// Use the object's specialization when a class member declared a
    /// parameterized key. Unresolved keys keep the conservative path.
    fn trace_known_class_key(&self, parent: &str, named_type: Option<&str>) -> Option<bool> {
        if let Some(owner) = self.live_storage_owner(parent) {
            let (_, member) = parent.split_once('#')?;
            if member.contains(['[', '.']) {
                return named_type
                    .filter(|name| !self.class_member_names().params.contains(*name))
                    .and_then(|name| self.module.classes.contains_key(name).then_some(true));
            }
            let object = self.heap[owner].as_ref()?;
            let mut class = Some(object.class_name.as_str());
            let mut chain = Vec::new();
            while let Some(cd) = class.and_then(|name| self.module.classes.get(name)) {
                if chain.len() == 64 {
                    return None;
                }
                chain.push(cd.as_ref());
                if let Some(&string_keyed) = cd.assoc_properties.get(member) {
                    if string_keyed {
                        return Some(false);
                    }
                    let Some(key_type) = cd.assoc_key_types.get(member) else {
                        return cd.assoc_index_props.contains_key(member).then_some(false);
                    };
                    let is_parameter = cd.type_param_names.contains(key_type);
                    if !is_parameter {
                        return self.module.classes.contains_key(key_type).then_some(true);
                    }
                    let concrete = Self::carried_type_binding(
                        &object.type_bindings,
                        &chain,
                        chain.len() - 1,
                        key_type,
                    )?;
                    let concrete = concrete.split('#').next()?;
                    if self.module.classes.contains_key(concrete) {
                        return Some(true);
                    }
                    return Self::gc_builtin_non_handle_type(concrete).then_some(false);
                }
                class = cd.extends.as_deref();
            }
            return None;
        }
        let name = named_type.or_else(|| {
            self.module
                .assoc_key_type_names
                .get(parent)
                .map(String::as_str)
        })?;
        self.module.classes.contains_key(name).then_some(true)
    }

    /// The storage spelling contains both a path and its selectors. Positional,
    /// string and integral selectors cannot hold class handles (§7.4, §7.8).
    /// An unknown selector type stays conservative, including class keys and
    /// type parameters whose concrete key type is unavailable here.
    fn trace_storage_name(&self, trace: &mut HandleTrace, name: &str) {
        let Some(first) = name.find('[') else {
            trace.elem_run_base(name);
            return;
        };
        if first == 0 {
            trace.elem_run_base(name);
            return;
        }
        trace.elem_run_base(&name[..first]);
        let mut open = first;
        loop {
            let key = &name[open + 1..];
            let Some(spelling) = Self::assoc_first_key_seg(key) else {
                trace.elem_run_base(key);
                return;
            };
            let parent = &name[..open];
            let dim = self
                .coll_shape_at(parent)
                .and_then(|(shape, depth)| shape.dims.get(depth));
            let scalar = match dim {
                Some(Dim::Assoc(KeyKind::Unknown | KeyKind::Typed(_))) => false,
                Some(_) => true,
                None => match self.module.associative_arrays.get(parent) {
                    Some(&string_keyed) => {
                        string_keyed
                            || self.live_storage_owner(parent).is_none()
                                && !parent.contains("::")
                                && self.module.assoc_index_widths.contains_key(parent)
                                && !self.module.assoc_key_type_names.contains_key(parent)
                    }
                    None => {
                        self.module.dynamic_arrays.contains(parent)
                            || self.module.queue_vars.contains(parent)
                            || self.module.arrays.contains_key(parent)
                            || self.module.arrays_nd.contains_key(parent)
                            || self.module.arrays_2d.contains_key(parent)
                    }
                },
            };
            if !scalar {
                let named_type = match dim {
                    Some(Dim::Assoc(KeyKind::Typed(name))) => Some(name.as_str()),
                    _ => None,
                };
                match self.trace_known_class_key(parent, named_type) {
                    Some(true) => {
                        // Older registrations may store a class key in the
                        // fixed-width hex spelling of a wide index slot.
                        if let Ok(handle) = spelling.parse::<u64>() {
                            trace.candidate(handle);
                        }
                        if self
                            .assoc_index_width_for(parent)
                            .is_some_and(|(width, _)| {
                                width > 64 && spelling.len() == width.div_ceil(4) as usize
                            })
                            && let Ok(handle) = u64::from_str_radix(spelling, 16)
                        {
                            trace.candidate(handle);
                        }
                    }
                    Some(false) => {}
                    None => trace.trace_storage_key(spelling),
                }
            }
            // A nested member can have its own collection. Classify its
            // selectors separately without retracing earlier positional keys.
            let rest = open + spelling.len() + 2;
            let Some(next) = name[rest..].find('[') else {
                break;
            };
            open = rest + next;
        }
    }

    fn trace_named_storage<'a>(
        &self,
        trace: &mut HandleTrace,
        name: &str,
        values: impl IntoIterator<Item = &'a Value>,
    ) -> Result<(), TraceError> {
        self.trace_storage_name(trace, name);
        if self.is_collection_size_slot(name) {
            return Ok(());
        }
        if let Some(owner) = self.live_storage_owner(name) {
            if self.class_property_has_no_handles(owner, name.split_once('#').unwrap().1) {
                return Ok(());
            }
        } else if self.named_storage_has_no_handles(name) {
            return Ok(());
        }
        for value in values {
            trace.trace_value(value)?;
        }
        Ok(())
    }

    fn is_collection_size_slot(&self, name: &str) -> bool {
        let Some(base) = name.strip_suffix(".size") else {
            return false;
        };
        self.module.dynamic_arrays.contains(base)
            || self.module.queue_vars.contains(base)
            || self.coll_shape_at(base).is_some_and(|(shape, depth)| {
                matches!(shape.dims.get(depth), Some(Dim::Dyn | Dim::Queue))
            })
    }

    fn named_storage_has_no_handles(&self, name: &str) -> bool {
        if self.is_collection_size_slot(name) {
            return true;
        }
        let declaration = self.module.var_decl_types.get(name).or_else(|| {
            let first = name.find('[')?;
            // A selected aggregate member may have a different declaration.
            Self::select_count(&name[first..])?;
            self.module.var_decl_types.get(&name[..first])
        });
        declaration.is_some_and(|dt| {
            Self::type_is_plain_scalar(dt)
                || matches!(dt, DataType::Enum(_) | DataType::IntegerVector { .. })
        })
    }

    fn gc_builtin_non_handle_type(name: &str) -> bool {
        matches!(
            name,
            "string"
                | "bit"
                | "logic"
                | "reg"
                | "byte"
                | "shortint"
                | "int"
                | "longint"
                | "integer"
                | "time"
                | "real"
                | "shortreal"
                | "realtime"
        )
    }

    fn class_property_has_no_handles(&self, handle: usize, property: &str) -> bool {
        let property = property.split(['[', '.']).next().unwrap_or(property);
        let Some(object) = self.heap.get(handle).and_then(Option::as_ref) else {
            return false;
        };
        let mut class = Some(object.class_name.as_str());
        for depth in 0..64 {
            let Some(cd) = class.and_then(|c| self.module.classes.get(c)) else {
                return false;
            };
            if let Some(data_type) = cd.property_types.get(property) {
                if let DataType::TypeReference { name, .. } = data_type {
                    let param = &name.name.name;
                    // Resolve a parameter through the instance's extends
                    // bindings. A global typedef of the same name is unrelated.
                    if !name.has_scope() && cd.type_param_names.contains(param) {
                        let mut chain = Vec::with_capacity(depth + 1);
                        let mut cursor = Some(object.class_name.as_str());
                        for _ in 0..=depth {
                            let Some(def) = cursor.and_then(|c| self.module.classes.get(c)) else {
                                return false;
                            };
                            chain.push(def.as_ref());
                            cursor = def.extends.as_deref();
                        }
                        return Self::carried_type_binding(
                            &object.type_bindings,
                            &chain,
                            depth,
                            param,
                        )
                        .is_some_and(|bound| {
                            Self::gc_builtin_non_handle_type(&bound)
                                && !self.module.classes.contains_key(&bound)
                        });
                    }
                    return false;
                }
                return matches!(
                    data_type,
                    DataType::IntegerAtom { .. }
                        | DataType::IntegerVector { .. }
                        | DataType::Real { .. }
                        | DataType::Enum(_)
                        | DataType::Implicit { .. }
                        | DataType::Simple {
                            kind: crate::ast::types::SimpleType::String,
                            ..
                        }
                );
            }
            class = cd.extends.as_deref();
        }
        false
    }
}

impl Simulator {
    /// Sweep only slots whose unreachable status has already been established
    /// (`maybe_collect_class_heap`). Keep handle indices monotonic: stale
    /// numeric comparisons must never alias a subsequently allocated object.
    #[cfg(test)]
    pub(super) fn release_unreachable_class_storage(&mut self, dead: &[bool]) -> usize {
        assert_eq!(dead.len(), self.heap.len());
        assert!(!dead.first().copied().unwrap_or(false));
        let list: Vec<usize> = (0..dead.len()).filter(|&h| dead[h]).collect();
        self.release_class_storage(&list)
    }

    fn release_class_storage(&mut self, dead_list: &[usize]) -> usize {
        if dead_list.is_empty() {
            return 0;
        }
        // Only touched event alias groups need a membership test. Build
        // the set on demand, sized to the slots actually being swept.
        let dead_set = std::cell::OnceCell::<HashSet<usize>>::new();
        let dead_handle = |handle| {
            if dead_list.len() <= 8 {
                dead_list.contains(&handle)
            } else {
                dead_set
                    .get_or_init(|| dead_list.iter().copied().collect())
                    .contains(&handle)
            }
        };
        for &handle in dead_list {
            let owner = StorageOwner::Class(handle);
            self.signals.release_storage_owner(owner);
            self.widths.release_storage_owner(owner);
            self.release_owned_metadata(owner);
            self.release_mailbox_storage(handle);
        }
        macro_rules! handles {
            ($($field:ident),* $(,)?) => {
                $(if !self.$field.is_empty() {
                    for handle in dead_list {
                        self.$field.remove(handle);
                    }
                })*
            };
        }
        handles!(
            obj_rng,
            rand_mode_disabled,
            constraint_mode_disabled,
            semaphores,
            semaphore_get_waiters,
            pending_class_initializers,
            prop_vif_bindings
        );
        let mut affected_event_ids = HashSet::default();
        for &owner in dead_list {
            self.virtual_iface_bindings.remove_owner(owner, |_, _| {});
            self.randc_used.remove_owner(owner, |_, _| {});
            self.randc_pending.remove_owner(owner, |_, _| {});
            self.dist_decks.remove_owner(owner, |_, _| {});
            self.event_obj_ids.remove_owner(owner, |_, id| {
                affected_event_ids.insert(id);
            });
        }
        for id in affected_event_ids {
            if let Some(slots) = self.event_obj_slots.get_mut(&id) {
                slots.retain(|(h, _)| !dead_handle(*h));
                if slots.is_empty() {
                    self.event_obj_slots.remove(&id);
                }
            }
        }
        let mut released = 0;
        for &handle in dead_list {
            if self.heap.get_mut(handle).and_then(Option::take).is_some() {
                released += 1;
            }
        }
        released
    }
}

impl Simulator {
    /// `XEZIM_GROWTH_TRACE=<ticks>`: every <ticks> of simulated time, print
    /// the sizes of the tables whose growth slows a long run down.
    pub(super) fn growth_trace(&mut self) {
        static EVERY: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        let Some(every) = *EVERY.get_or_init(|| {
            std::env::var("XEZIM_GROWTH_TRACE").ok()?.parse::<u64>().ok()
        }) else {
            return;
        };
        if self.time < self.growth_trace_next {
            return;
        }
        self.growth_trace_next = self.time + every.max(1);
        let frame_keys: usize = self
            .process_contexts
            .values()
            .map(|c| c.local_stack.iter().map(|f| f.len()).sum::<usize>())
            .sum();
        eprintln!(
            "[GROWTH] t={} heap_slots={} live={} gc_runs={} signals={} contexts={} frame_keys={} \
             event_waiters={} instance_event_waiters={} condition_waiters={} join_waiters={}",
            self.time,
            self.heap.len(),
            self.heap.iter().filter(|o| o.is_some()).count(),
            self.class_gc_runs,
            self.signals.len(),
            self.process_contexts.len(),
            frame_keys,
            self.event_waiters.len(),
            self.instance_event_waiters.len(),
            self.condition_waiters.len(),
            self.join_waiters.len(),
        );
    }
}
