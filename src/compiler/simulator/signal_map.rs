//! The signal map, element runs, waveform variable kinds, forces and name-table helpers.

use super::heap_gc::{ClassStorageKeys, StorageOwner};
use super::*;
use std::ops::Bound::{Included, Unbounded};

/// `h` without its last segment. The derived `Clone` copies the resolution
/// caches, which belong to the full path; left in place, the prefix resolved
/// as the whole name (`u.sig` evaluated `u` as the signal `u.sig`).
pub(super) fn hier_prefix(
    h: &crate::ast::expr::HierarchicalIdentifier,
) -> crate::ast::expr::HierarchicalIdentifier {
    let mut p = h.clone();
    p.path.pop();
    p.cached_signal_id = crate::ast::cache::SignalCache::new(None);
    p.cached_resolved_name = crate::ast::cache::NameCache::new();
    p
}

#[inline]
pub(super) fn resolve_array_elem_id(
    name: &str,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
) -> Option<usize> {
    let bytes = name.as_bytes();
    if bytes.last() != Some(&b']') {
        return None;
    }
    let brack = name.rfind('[')?;
    let base = &name[..brack];
    let idx_str = &name[brack + 1..name.len() - 1];
    let idx: i64 = idx_str.parse().ok()?;
    let &(first_id, lo, hi) = array_first_id.get(base)?;
    if idx < lo || idx > hi {
        return None;
    }
    Some(first_id + (idx - lo) as usize)
}

#[inline(always)]
/// Element width of a bytecode array operand, so an out-of-range or x/z-index
/// READ can yield x at the ELEMENT width. A 1-bit x zero-extends into a wider
/// element (`0000000x`), which silently turns "unknown" into "almost all
/// zero" — an AES S-box indexed by an uninitialized input must stay all-x.
pub(super) fn bytecode_array_elem_width(
    array: &super::bytecode::ArrayOperand,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
    signal_table: &[Value],
) -> u32 {
    bytecode_array_first_id(array, array_first_id)
        .and_then(|f| signal_table.get(f))
        .map(|v| v.width)
        .unwrap_or(1)
        .max(1)
}

/// Id of element `lo` of a bytecode array operand, when it resolves.
pub(super) fn bytecode_array_first_id(
    array: &super::bytecode::ArrayOperand,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
) -> Option<usize> {
    match array {
        super::bytecode::ArrayOperand::Dense { first_id, .. } => Some(*first_id),
        super::bytecode::ArrayOperand::Named(name) => {
            array_first_id.get(name.as_str()).map(|&(f, _, _)| f)
        }
    }
}

/// Every array access in a compiled block lands here; on a CPU core all of
/// them are `Dense` (a bounds check and an add), so that arm is inlined
/// at the thirteen call sites and only the name-keyed lookup stays out of
/// line.
#[inline(always)]
pub(super) fn resolve_bytecode_array_elem(
    array: &super::bytecode::ArrayOperand,
    idx: i64,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
    signal_name_to_id: &NameMap,
) -> Option<usize> {
    if let super::bytecode::ArrayOperand::Dense {
        first_id, lo, hi, ..
    } = array
    {
        return if idx >= *lo && idx <= *hi {
            Some(*first_id + (idx - *lo) as usize)
        } else {
            None
        };
    }
    resolve_bytecode_array_elem_named(array, idx, array_first_id, signal_name_to_id)
}

#[inline(never)]
fn resolve_bytecode_array_elem_named(
    array: &super::bytecode::ArrayOperand,
    idx: i64,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
    signal_name_to_id: &NameMap,
) -> Option<usize> {
    match array {
        super::bytecode::ArrayOperand::Dense { .. } => None,
        super::bytecode::ArrayOperand::Named(name) => array_first_id
            .get(name.as_str())
            .and_then(|&(first, lo, hi)| {
                if idx >= lo && idx <= hi {
                    Some(first + (idx - lo) as usize)
                } else {
                    None
                }
            })
            .or_else(|| {
                // Diagnostic for the slow by-name fallback (format! + String
                // hash per access — measured ~9% of c906 memcpy cycles when a
                // hot array lands here). XEZIM_DEBUG_NAMED_ARRAYS=1 prints
                // each distinct array name once.
                if std::env::var_os("XEZIM_DEBUG_NAMED_ARRAYS").is_some() {
                    use std::sync::{Mutex, OnceLock};
                    static SEEN: OnceLock<Mutex<std::collections::HashSet<String>>> =
                        OnceLock::new();
                    let seen = SEEN.get_or_init(|| Mutex::new(Default::default()));
                    if seen.lock().unwrap().insert(name.clone()) {
                        eprintln!("[NAMED-ARRAY] slow-path array operand: {}", name);
                    }
                }
                let elem_name = format!("{}[{}]", name, idx);
                signal_name_to_id.get(elem_name.as_str()).copied()
            }),
    }
}

/// IEEE 1800-2017 §21.7.2.1 `var_type` of a dumped object. Every `$var` used to
/// be hardcoded `wire`, which mislabels every variable in the design and hides
/// `event` semantics from the viewer entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum VcdVarKind {
    Wire,
    Reg,
    Integer,
    Time,
    Real,
    Event,
    Parameter,
}

/// One expression-backed procedural continuous assignment/force.
///
/// Resolved signal reads carry the generation observed at the previous RHS
/// evaluation. Reads that cannot be mapped to compact signal storage retain a
/// conservative fallback so class/runtime-map expressions keep their prior
/// semantics.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ActiveForceExpr {
    pub(super) key: String,
    pub(super) lvalue: Expression,
    pub(super) rvalue: Expression,
    pub(super) hint: Option<String>,
    pub(super) read_signal_ids: Vec<usize>,
    pub(super) read_epochs: Vec<u64>,
    pub(super) has_unresolved_reads: bool,
    /// §10.6.2 bit/part-select force on a net (`partial_forces`): the reads
    /// are the net's drivers' and the forcing expressions'; `lvalue` is the
    /// whole net.
    pub(super) partial: bool,
}

/// IEEE 1800-2023 §10.6.2: the forced bits of a net under bit/part-select
/// `force`. `bits[i]` names the piece (forcing expression) and the bit
/// within it that bit `i` takes; `None` bits follow the net's drivers.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PartialForce {
    pub(super) bits: Vec<Option<(usize, u32)>>,
    /// (rvalue, select width, resolution hint) per `force` statement.
    pub(super) pieces: Vec<(Expression, u32, Option<String>)>,
}

impl VcdVarKind {
    /// The §21.7.2.1 `var_type` keyword.
    pub(super) fn keyword(self) -> &'static str {
        match self {
            VcdVarKind::Wire => "wire",
            VcdVarKind::Reg => "reg",
            VcdVarKind::Integer => "integer",
            VcdVarKind::Time => "time",
            VcdVarKind::Real => "real",
            VcdVarKind::Event => "event",
            VcdVarKind::Parameter => "parameter",
        }
    }
}

/// Sparse elements are grouped by outermost collection. Each bucket owns the
/// values and supplies the element index, including nested rows and members.
/// Contiguous positional elements live in `ElemRun` (§7.5, §7.10).
pub struct SignalMap {
    map: SparseSignals,
    /// Per base name, a stamp that changes whenever an element key is added
    /// to or removed from the base (absent = no element keys), so a derived
    /// view of a base's key set can be cached until it changes.
    elem_gens: HashMap<String, u64>,
    elem_gen_counter: u64,
    /// The dense element runs, by collection name (`q`, `12#m_q`, `@q#3`,
    /// `u.q`, a row `aq[3]`).
    runs: HashMap<String, ElemRun>,
    /// Elements held in `runs`, so `len()` counts them as keys.
    run_keys: usize,
    /// The names in `runs` that contain `[` themselves, by base name: the
    /// runs whose keys the element index files under another name.
    nested_runs: HashMap<String, Vec<String>>,
    /// Derived from the map and runs. Restores rebuild it on first use,
    /// keeping the checkpoint representation unchanged.
    class_keys: Option<(ClassStorageKeys, ClassStorageKeys)>,
}

// A complete canonical decimal selector fits in an i64 or u64. Reserve
// i128::MIN for the second component of a one-dimensional key.
type NumericSelectors = (i128, i128);

fn decimal_selector(suffix: &str) -> Option<(i128, &str)> {
    let bytes = suffix.as_bytes().strip_prefix(b"[")?;
    let negative = bytes.first() == Some(&b'-');
    let start = usize::from(negative);
    let digits = &bytes[start..];
    let mut magnitude = 0u64;
    for (i, &byte) in digits.iter().enumerate() {
        if byte == b']' {
            if i == 0 || (digits[0] == b'0' && (i > 1 || negative)) {
                return None;
            }
            let value = i128::from(magnitude);
            return Some((
                if negative { -value } else { value },
                &suffix[2 + start + i..],
            ));
        }
        let digit = byte.wrapping_sub(b'0');
        if digit > 9 {
            return None;
        }
        magnitude = magnitude.checked_mul(10)?.checked_add(u64::from(digit))?;
    }
    None
}

fn numeric_selectors(suffix: &str) -> Option<NumericSelectors> {
    let (first, rest) = decimal_selector(suffix)?;
    if rest.is_empty() {
        return Some((first, i128::MIN));
    }
    let (second, rest) = decimal_selector(rest)?;
    rest.is_empty().then_some((first, second))
}

#[derive(Default)]
struct SparseElements {
    numeric: BTreeMap<NumericSelectors, (String, Value)>,
    named: BTreeMap<String, Value>,
}

impl SparseElements {
    fn len(&self) -> usize {
        self.numeric.len() + self.named.len()
    }

    fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.numeric
            .values()
            .map(|(k, v)| (k, v))
            .chain(self.named.iter())
    }

    fn keys_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
        base_len: usize,
    ) -> impl Iterator<Item = &'a String> {
        let suffix = &prefix[base_len..];
        let numeric_range = match decimal_selector(suffix) {
            Some((first, tail)) => match decimal_selector(tail) {
                Some((second, _)) => Some(((first, second), (first, second))),
                None => Some(((first, i128::MIN), (first, i128::MAX))),
            },
            // A complete noncanonical selector belongs to the named tree.
            None if suffix.contains(']') => None,
            None => Some(((i128::MIN, i128::MIN), (i128::MAX, i128::MAX))),
        };
        numeric_range
            .into_iter()
            .flat_map(|(first, last)| self.numeric.range(first..=last))
            .map(|(_, (key, _))| key)
            .filter(move |key| key.starts_with(prefix))
            .chain(
                self.named
                    .range::<str, _>((Included(prefix), Unbounded))
                    .map(|(key, _)| key)
                    .take_while(move |key| key.starts_with(prefix)),
            )
    }
}

#[derive(Default)]
struct SparseSignals {
    plain: HashMap<String, Value>,
    indexed: HashMap<String, SparseElements>,
    indexed_len: usize,
}

impl SparseSignals {
    #[inline]
    fn get(&self, key: &str) -> Option<&Value> {
        if self.indexed.is_empty() {
            return self.plain.get(key);
        }
        match SignalMap::base_of(key) {
            Some(base) => {
                let entries = self.indexed.get(base)?;
                match numeric_selectors(&key[base.len()..]) {
                    Some(index) => entries.numeric.get(&index).map(|(_, value)| value),
                    None => entries.named.get(key),
                }
            }
            None => self.plain.get(key),
        }
    }

    #[inline]
    fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match SignalMap::base_of(key) {
            Some(base) => {
                let entries = self.indexed.get_mut(base)?;
                match numeric_selectors(&key[base.len()..]) {
                    Some(index) => entries.numeric.get_mut(&index).map(|(_, value)| value),
                    None => entries.named.get_mut(key),
                }
            }
            None => self.plain.get_mut(key),
        }
    }

    fn insert(&mut self, key: String, value: Value, on_new: impl FnOnce(&str)) -> Option<Value> {
        use std::collections::{btree_map, hash_map};

        let Some(base) = SignalMap::base_of(&key) else {
            return match self.plain.entry(key) {
                hash_map::Entry::Occupied(mut entry) => Some(entry.insert(value)),
                hash_map::Entry::Vacant(entry) => {
                    on_new(entry.key());
                    entry.insert(value);
                    None
                }
            };
        };
        let entries = match self.indexed.get_mut(base) {
            Some(entries) => entries,
            None => self.indexed.entry(base.to_owned()).or_default(),
        };
        match numeric_selectors(&key[base.len()..]) {
            Some(index) => match entries.numeric.entry(index) {
                btree_map::Entry::Occupied(mut entry) => {
                    return Some(std::mem::replace(&mut entry.get_mut().1, value));
                }
                btree_map::Entry::Vacant(entry) => {
                    on_new(&key);
                    entry.insert((key, value));
                }
            },
            None => match entries.named.entry(key) {
                btree_map::Entry::Occupied(mut entry) => return Some(entry.insert(value)),
                btree_map::Entry::Vacant(entry) => {
                    on_new(entry.key());
                    entry.insert(value);
                }
            },
        }
        self.indexed_len += 1;
        None
    }

    fn remove(&mut self, key: &str) -> Option<Value> {
        let Some(base) = SignalMap::base_of(key) else {
            return self.plain.remove(key);
        };
        let entries = self.indexed.get_mut(base)?;
        let value = match numeric_selectors(&key[base.len()..]) {
            Some(index) => entries.numeric.remove(&index).map(|(_, value)| value),
            None => entries.named.remove(key),
        }?;
        self.indexed_len -= 1;
        if entries.len() == 0 {
            self.indexed.remove(base);
        }
        Some(value)
    }

    fn len(&self) -> usize {
        self.plain.len() + self.indexed_len
    }

    fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.plain
            .iter()
            .chain(self.indexed.values().flat_map(SparseElements::iter))
    }

    fn keys(&self) -> impl Iterator<Item = &String> {
        self.iter().map(|(key, _)| key)
    }

    fn keys_under(&self, base: &str) -> impl Iterator<Item = &String> {
        self.indexed
            .get(base)
            .into_iter()
            .flat_map(|entries| entries.iter().map(|(key, _)| key))
    }

    fn keys_with_prefix<'a>(
        &'a self,
        base: &str,
        prefix: &'a str,
    ) -> impl Iterator<Item = &'a String> {
        let base_len = base.len();
        self.indexed
            .get(base)
            .into_iter()
            .flat_map(move |entries| entries.keys_with_prefix(prefix, base_len))
    }
}

// Keep the checkpoint layout used before buckets held both values and keys.
// The redundant index is emitted on save for compatibility with older readers.
impl serde::Serialize for SparseSignals {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (key, value) in self.iter() {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

struct CheckpointElementIndex<'a>(&'a SparseSignals);

impl serde::Serialize for CheckpointElementIndex<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        struct Keys<'a>(&'a SparseElements);

        impl serde::Serialize for Keys<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
                for (key, _) in self.0.iter() {
                    seq.serialize_element(key)?;
                }
                seq.end()
            }
        }

        let mut map = serializer.serialize_map(Some(self.0.indexed.len()))?;
        for (base, entries) in &self.0.indexed {
            map.serialize_entry(base, &Keys(entries))?;
        }
        map.end()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename = "SignalMap")]
struct SignalMapCheckpoint<M, E, G, R, N> {
    map: M,
    elems: E,
    elem_gens: G,
    elem_gen_counter: u64,
    runs: R,
    run_keys: usize,
    nested_runs: N,
}

impl serde::Serialize for SignalMap {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SignalMapCheckpoint {
            map: &self.map,
            elems: CheckpointElementIndex(&self.map),
            elem_gens: &self.elem_gens,
            elem_gen_counter: self.elem_gen_counter,
            runs: &self.runs,
            run_keys: self.run_keys,
            nested_runs: &self.nested_runs,
        }
        .serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for SignalMap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let stored: SignalMapCheckpoint<
            HashMap<String, Value>,
            HashMap<String, std::collections::HashSet<String>>,
            _,
            _,
            _,
        > = serde::Deserialize::deserialize(deserializer)?;
        let mut map = SparseSignals::default();
        for (key, value) in stored.map {
            map.insert(key, value, |_| {});
        }
        Ok(Self {
            map,
            elem_gens: stored.elem_gens,
            elem_gen_counter: stored.elem_gen_counter,
            runs: stored.runs,
            run_keys: stored.run_keys,
            nested_runs: stored.nested_runs,
            class_keys: None,
        })
    }
}

/// The elements `name[start]`, `name[start + 1]`, ... of one collection.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct ElemRun {
    /// Index of `items[0]` (meaningless while `items` is empty).
    start: u64,
    items: VecDeque<Value>,
    /// `name.size` is stored: the name is a queue or dynamic array, and the
    /// run is kept while empty.
    sized: bool,
    /// Map-held keys `name[<index>]` outside the run.
    strays: u32,
}

impl ElemRun {
    #[inline]
    pub(super) fn end(&self) -> u64 {
        self.start + self.items.len() as u64
    }

    #[inline]
    pub(super) fn slot(&self, i: u64) -> Option<usize> {
        (i >= self.start && i < self.end()).then(|| (i - self.start) as usize)
    }
}

/// `name[<n>]` with `<n>` a canonical decimal index: the collection name and
/// the index. Any other spelling (`a[01]`, `a[-1]`, `a["k"]`, `a[1].f`) is
/// not an element of a run.
#[inline]
pub(super) fn split_elem_key(k: &str) -> Option<(&str, u64)> {
    let b = k.as_bytes();
    let n = b.len();
    if n < 4 || b[n - 1] != b']' {
        return None;
    }
    let mut i = n - 1;
    while i > 0 && b[i - 1].is_ascii_digit() {
        i -= 1;
    }
    let digits = n - 1 - i;
    if digits == 0 || digits > 9 || i < 2 || b[i - 1] != b'[' {
        return None;
    }
    if digits > 1 && b[i] == b'0' {
        return None;
    }
    let mut v = 0u64;
    for &d in &b[i..n - 1] {
        v = v * 10 + (d - b'0') as u64;
    }
    Some((&k[..i - 1], v))
}

impl SignalMap {
    pub fn new() -> Self {
        SignalMap {
            map: SparseSignals::default(),
            elem_gens: HashMap::default(),
            elem_gen_counter: 0,
            runs: HashMap::default(),
            run_keys: 0,
            nested_runs: HashMap::default(),
            class_keys: Some(Default::default()),
        }
    }

    pub(super) fn base_of(k: &str) -> Option<&str> {
        k.find('[').map(|i| &k[..i])
    }

    /// The base name the element index files `name`'s elements under.
    #[inline]
    fn outer_of(name: &str) -> &str {
        Self::base_of(name).unwrap_or(name)
    }

    #[inline]
    pub fn get(&self, k: &str) -> Option<&Value> {
        if !self.runs.is_empty()
            && let Some((name, i)) = split_elem_key(k)
            && let Some(run) = self.runs.get(name)
            && let Some(s) = run.slot(i)
        {
            return Some(&run.items[s]);
        }
        self.map.get(k)
    }

    #[inline]
    pub fn contains_key(&self, k: &str) -> bool {
        self.get(k).is_some()
    }

    /// Stored keys, run-held elements included.
    pub fn len(&self) -> usize {
        self.map.len() + self.run_keys
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every stored key with its value, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (std::borrow::Cow<'_, str>, &Value)> + '_ {
        self.map
            .iter()
            .map(|(k, v)| (std::borrow::Cow::Borrowed(k.as_str()), v))
            .chain(self.runs.iter().flat_map(|(name, run)| {
                run.items.iter().enumerate().map(move |(s, v)| {
                    (
                        std::borrow::Cow::Owned(format!("{}[{}]", name, run.start + s as u64)),
                        v,
                    )
                })
            }))
    }

    /// The entries the map itself holds: every key but a run's elements
    /// (`elem_runs`).
    pub fn map_entries(&self) -> impl Iterator<Item = (&String, &Value)> + '_ {
        self.map.iter()
    }

    /// The element runs: a collection name, the index of its first held
    /// element and the elements `name[first]`, `name[first + 1]`, ...
    pub fn elem_runs(&self) -> impl Iterator<Item = (&str, u64, &VecDeque<Value>)> + '_ {
        self.runs
            .iter()
            .filter(|(_, run)| !run.items.is_empty())
            .map(|(name, run)| (name.as_str(), run.start, &run.items))
    }

    pub(super) fn owner_storage(
        &self,
        owner: StorageOwner,
    ) -> (
        impl Iterator<Item = (&str, &Value)>,
        impl Iterator<Item = (&str, &VecDeque<Value>)>,
    ) {
        let (names, runs) = self.class_keys.as_ref().expect("runtime owner index");
        (
            names
                .names(owner)
                .filter_map(|name| self.map.get(name).map(|v| (name, v))),
            runs.names(owner)
                .filter_map(|name| self.runs.get(name).map(|r| (name, &r.items))),
        )
    }

    pub(super) fn rebuild_owner_index(&mut self) {
        if self.class_keys.is_none() {
            let mut keys = (ClassStorageKeys::default(), ClassStorageKeys::default());
            for name in self.map.keys() {
                keys.0.insert(name);
            }
            for name in self.runs.keys() {
                keys.1.insert(name);
            }
            self.class_keys = Some(keys);
        }
    }

    pub(super) fn release_storage_owner(&mut self, owner: StorageOwner) {
        self.rebuild_owner_index();
        let (map_keys, run_names) = self.class_keys.as_mut().unwrap();
        // Save both lists before editing the element index. Removing
        // '.size' or a stray may also drop a run.
        let keys = map_keys.take(owner);
        let runs = run_names.take(owner);
        if let Some(runs) = &runs {
            // Every nested run under an owned base has the same owner.
            // Drop that list once instead of filtering it per row.
            for name in runs {
                self.nested_runs.remove(Self::outer_of(name));
            }
        }
        for name in keys.into_iter().flatten() {
            self.remove(&name);
        }
        for name in runs.into_iter().flatten() {
            self.truncate_elem_run(&name, 0);
            self.drop_idle_run(&name);
        }
    }

    /// Release a group of runtime rows belonging to an owned collection.
    /// Deleting a row drops a dense run without materializing element names.
    pub(super) fn release_matching_storage(
        &mut self,
        owner: StorageOwner,
        discard: &impl Fn(&str) -> bool,
    ) {
        self.rebuild_owner_index();
        let (entries, runs) = self.class_keys.as_mut().unwrap();
        let entries = entries.take_matching(owner, discard);
        let runs = runs.take_matching(owner, discard);
        for name in entries {
            self.remove(&name);
        }
        for name in runs {
            self.truncate_elem_run(&name, 0);
            self.drop_idle_run(&name);
        }
    }

    /// Discard the dense suffix starting at `keep` without spelling its keys.
    pub(super) fn truncate_elem_run(&mut self, name: &str, keep: u64) {
        let Some(run) = self.runs.get_mut(name) else {
            return;
        };
        let len = run.items.len();
        let retained = keep.saturating_sub(run.start).min(len as u64) as usize;
        if retained == len {
            return;
        }
        note_store_write(signal_key_bits(name));
        run.items.truncate(retained);
        self.run_keys -= len - retained;
        self.drop_idle_run(name);
        self.stamp_base(Self::outer_of(name));
    }

    /// Drop the elements and any member or row storage beyond a sequence's
    /// retained prefix (§7.5.1, §7.10.2).
    pub(super) fn truncate_sequence(&mut self, name: &str, keep: u64) {
        self.truncate_elem_run(name, keep);
        let outside = |key: &str| {
            key.strip_prefix(name)
                .and_then(|tail| tail.strip_prefix('['))
                .and_then(|tail| tail.split_once(']'))
                .and_then(|(index, _)| index.parse::<u64>().ok())
                .is_some_and(|index| index >= keep)
        };
        let outer = Self::outer_of(name);
        let keys: Vec<_> = self
            .map
            .keys_under(outer)
            .filter(|key| outside(key))
            .cloned()
            .collect();
        for key in keys {
            self.remove(&key);
        }
        let rows: Vec<_> = self
            .nested_runs
            .get(outer)
            .into_iter()
            .flatten()
            .filter(|row| outside(row))
            .cloned()
            .collect();
        for row in rows {
            self.truncate_elem_run(&row, 0);
        }
    }

    /// Every stored key, in no particular order.
    pub fn keys(&self) -> impl Iterator<Item = std::borrow::Cow<'_, str>> + '_ {
        self.iter().map(|(k, _)| k)
    }

    /// A key was added under or removed from base `outer`: move its stamp,
    /// or drop it with the base's last key.
    fn stamp_base(&mut self, outer: &str) {
        if self.map.indexed.contains_key(outer) || self.base_has_run_keys(outer) {
            Self::advance_stamp(&mut self.elem_gens, &mut self.elem_gen_counter, outer);
        } else {
            self.elem_gens.remove(outer);
        }
    }

    fn advance_stamp(gens: &mut HashMap<String, u64>, counter: &mut u64, outer: &str) {
        *counter += 1;
        match gens.get_mut(outer) {
            Some(generation) => *generation = *counter,
            None => {
                gens.insert(outer.to_owned(), *counter);
            }
        }
    }

    fn base_has_run_keys(&self, outer: &str) -> bool {
        self.runs.get(outer).is_some_and(|r| !r.items.is_empty())
            || self.nested_runs.get(outer).is_some_and(|names| {
                names
                    .iter()
                    .any(|n| self.runs.get(n).is_some_and(|r| !r.items.is_empty()))
            })
    }

    fn unindex_key(&mut self, k: &str) {
        if let Some((keys, _)) = &mut self.class_keys {
            keys.remove(k);
        }
        if let Some(base) = Self::base_of(k) {
            self.stamp_base(base);
        }
    }

    fn insert_sparse(&mut self, key: String, value: Value) -> Option<Value> {
        self.map.insert(key, value, |key| {
            if let Some((keys, _)) = &mut self.class_keys {
                keys.insert(key);
            }
            if let Some(base) = Self::base_of(key) {
                Self::advance_stamp(&mut self.elem_gens, &mut self.elem_gen_counter, base);
            }
        })
    }

    pub fn insert(&mut self, k: String, v: Value) -> Option<Value> {
        note_store_write(signal_key_bits(&k));
        if !self.runs.is_empty()
            && let Some((name, i)) = split_elem_key(&k)
            && let Some(run) = self.runs.get_mut(name)
        {
            if let Some(s) = run.slot(i) {
                return Some(std::mem::replace(&mut run.items[s], v));
            }
            // A key the map already holds stays there.
            if run.strays == 0 || self.map.get(&k).is_none() {
                let adopted = if run.items.is_empty() {
                    run.start = i;
                    run.items.push_back(v);
                    None
                } else if i == run.end() {
                    run.items.push_back(v);
                    None
                } else if i + 1 == run.start {
                    run.start = i;
                    run.items.push_front(v);
                    None
                } else {
                    Some(v)
                };
                match adopted {
                    None => {
                        let absorb = run.strays > 0;
                        self.run_keys += 1;
                        let name = name.to_string();
                        if absorb {
                            self.absorb_strays(&name);
                        }
                        self.stamp_base(Self::outer_of(&name));
                        return None;
                    }
                    Some(v) => {
                        run.strays += 1;
                        return self.insert_sparse(k, v);
                    }
                }
            }
            return self.insert_sparse(k, v);
        }
        if let Some(name) = k.strip_suffix(".size") {
            let name = name.to_owned();
            let previous = self.insert_sparse(k, v);
            if previous.is_none() {
                self.register_element_run(&name, true);
            }
            return previous;
        }
        self.insert_sparse(k, v)
    }

    /// Store contiguous elements in a run. Only a queue or dynamic array
    /// with a `.size` key keeps its run after the last element is removed.
    pub(super) fn register_element_run(&mut self, name: &str, sized: bool) {
        if let Some(run) = self.runs.get_mut(name) {
            run.sized |= sized;
            return;
        }
        let outer = Self::outer_of(name);
        let mut held: Vec<u64> = self
            .map
            .keys_under(outer)
            .filter_map(|key| {
                split_elem_key(key)
                    .filter(|(n, _)| *n == name)
                    .map(|(_, i)| i)
            })
            .collect();
        held.sort_unstable();
        let mut run = ElemRun {
            sized,
            strays: held.len() as u32,
            ..ElemRun::default()
        };
        if let Some(&first) = held.first() {
            run.start = first;
        }
        if outer.len() != name.len() {
            self.nested_runs
                .entry(outer.to_string())
                .or_default()
                .push(name.to_string());
        }
        if let Some((_, runs)) = &mut self.class_keys {
            runs.insert(name);
        }
        self.runs.insert(name.to_string(), run);
        if !held.is_empty() {
            self.absorb_strays(name);
        }
    }

    /// Move the map-held keys adjacent to `name`'s run into it.
    fn absorb_strays(&mut self, name: &str) {
        loop {
            let Some(run) = self.runs.get(name) else {
                return;
            };
            if run.strays == 0 {
                return;
            }
            // An empty run restarts at the index it last began at.
            let empty = run.items.is_empty();
            let above = if empty { run.start } else { run.end() };
            let below = if empty {
                None
            } else {
                run.start.checked_sub(1)
            };
            let mut took = false;
            for (back, i) in [(true, Some(above)), (false, below)] {
                let Some(i) = i else { continue };
                let key = format!("{}[{}]", name, i);
                if let Some(v) = self.map.remove(&key) {
                    // The key stays stored: the base's stamp does not move.
                    if let Some((keys, _)) = &mut self.class_keys {
                        keys.remove(&key);
                    }
                    let run = self.runs.get_mut(name).unwrap();
                    if back {
                        if run.items.is_empty() {
                            run.start = i;
                        }
                        run.items.push_back(v);
                    } else {
                        run.start = i;
                        run.items.push_front(v);
                    }
                    run.strays -= 1;
                    self.run_keys += 1;
                    took = true;
                }
            }
            if !took {
                return;
            }
        }
    }

    /// Forget `name`'s run once it holds nothing and `name.size` is gone.
    fn drop_idle_run(&mut self, name: &str) {
        if self
            .runs
            .get(name)
            .is_some_and(|r| r.items.is_empty() && !r.sized && r.strays == 0)
        {
            self.runs.remove(name);
            if let Some((_, runs)) = &mut self.class_keys {
                runs.remove(name);
            }
            let outer = Self::outer_of(name);
            if outer.len() != name.len()
                && let Some(names) = self.nested_runs.get_mut(outer)
            {
                names.retain(|n| n != name);
                if names.is_empty() {
                    self.nested_runs.remove(outer);
                }
            }
        }
    }

    pub fn remove(&mut self, k: &str) -> Option<Value> {
        note_store_write(signal_key_bits(k));
        if !self.runs.is_empty()
            && let Some((name, i)) = split_elem_key(k)
            && let Some(run) = self.runs.get_mut(name)
        {
            if let Some(s) = run.slot(i) {
                let last = run.items.len() - 1;
                let v = if s == last {
                    run.items.pop_back()
                } else if s == 0 {
                    run.start += 1;
                    run.items.pop_front()
                } else {
                    // A hole: the shorter side leaves the run for the map.
                    let v = run.items.remove(s);
                    let (lo, moved): (u64, Vec<Value>) = if s <= last / 2 {
                        let lo = run.start;
                        run.start = i + 1;
                        (lo, run.items.drain(..s).collect())
                    } else {
                        (i + 1, run.items.drain(s..).collect())
                    };
                    run.strays += moved.len() as u32;
                    self.run_keys -= moved.len();
                    for (j, mv) in moved.into_iter().enumerate() {
                        let key = format!("{}[{}]", name, lo + j as u64);
                        self.insert_sparse(key, mv);
                    }
                    v
                };
                self.run_keys -= 1;
                self.drop_idle_run(name);
                self.stamp_base(Self::outer_of(name));
                return v;
            }
            let r = self.map.remove(k);
            if r.is_some() {
                run.strays -= 1;
                self.unindex_key(k);
                self.drop_idle_run(name);
            }
            return r;
        }
        let r = self.map.remove(k);
        if r.is_some() {
            self.unindex_key(k);
            if let Some(name) = k.strip_suffix(".size")
                && let Some(run) = self.runs.get_mut(name)
            {
                run.sized = false;
                self.drop_idle_run(name);
            }
        }
        r
    }

    /// An in-place edit of one stored value, noted under its key's names.
    pub fn get_mut(&mut self, k: &str) -> Option<&mut Value> {
        note_store_write(signal_key_bits(k));
        if !self.runs.is_empty()
            && let Some((name, i)) = split_elem_key(k)
            && let Some(run) = self.runs.get_mut(name)
            && let Some(s) = run.slot(i)
        {
            return Some(&mut run.items[s]);
        }
        self.map.get_mut(k)
    }

    /// The runs whose keys the element index files under `outer`.
    fn runs_under<'a>(&'a self, outer: &'a str) -> impl Iterator<Item = (&'a str, &'a ElemRun)> {
        self.runs
            .get(outer)
            .map(|r| (outer, r))
            .into_iter()
            .chain(
                self.nested_runs
                    .get(outer)
                    .into_iter()
                    .flatten()
                    .filter_map(|n| self.runs.get(n).map(|r| (n.as_str(), r))),
            )
            .filter(|(_, r)| !r.items.is_empty())
    }

    /// Whether every key `name[i]` begins with `prefix`, which ends in `[`.
    #[inline]
    fn run_under_prefix(name: &str, prefix: &str) -> bool {
        name.starts_with(prefix) || (prefix.len() == name.len() + 1 && prefix.starts_with(name))
    }

    /// All stored keys beginning with `prefix`, which must end with `[`.
    /// Served from the element index: only the owning base name's keys are
    /// touched, never the whole map. Handles nested-bracket prefixes
    /// (`q[0].f[`) by indexing on the OUTERMOST base and filtering.
    /// A run's keys come highest index first, so a caller removing them in
    /// this order shrinks the run from its end.
    pub fn keys_with_elem_prefix(&self, prefix: &str) -> Vec<String> {
        debug_assert!(prefix.ends_with('['));
        self.elem_keys_with_prefix(prefix)
            .map(|k| k.into_owned())
            .collect()
    }

    /// Keys of every `base[...]` element currently stored, unordered (a
    /// run's keys highest index first, see `keys_with_elem_prefix`).
    pub fn elem_keys(&self, base: &str) -> Vec<String> {
        let mut keys: Vec<String> = self.map.keys_under(base).cloned().collect();
        // The index files nothing under a name with a bracket in it.
        if !base.contains('[') {
            for (name, run) in self.runs_under(base) {
                keys.extend(
                    (run.start..run.end())
                        .rev()
                        .map(|i| format!("{}[{}]", name, i)),
                );
            }
        }
        keys
    }

    /// Borrowing form of `keys_with_elem_prefix`: the element keys under
    /// `prefix` (which must end in `[`), cloning none of the map-held ones.
    pub fn elem_keys_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = std::borrow::Cow<'a, str>> + 'a {
        let base = &prefix[..prefix.len().saturating_sub(1)];
        let outer = Self::base_of(base).unwrap_or(base);
        self.map
            .keys_with_prefix(outer, prefix)
            .map(|key| std::borrow::Cow::Borrowed(key.as_str()))
            .chain(
                self.runs_under(outer)
                    .filter(move |(name, _)| Self::run_under_prefix(name, prefix))
                    .flat_map(|(name, run)| {
                        (run.start..run.end())
                            .rev()
                            .map(move |i| std::borrow::Cow::Owned(format!("{}[{}]", name, i)))
                    }),
            )
    }

    /// The element-key stamp of the outermost base of `prefix` (which ends
    /// in `[`): equal stamps mean an unchanged key set under that base.
    pub fn elem_gen_for_prefix(&self, prefix: &str) -> u64 {
        let base = &prefix[..prefix.len().saturating_sub(1)];
        let outer = Self::base_of(base).unwrap_or(base);
        self.elem_gens.get(outer).copied().unwrap_or(0)
    }

    /// Is any key under the outermost base of `prefix` prefixed by it?
    /// Every key that contains `[` is indexed under the text before its
    /// FIRST `[`, so `obj[k][j]` and `obj[k].field` are both found under
    /// `obj`; this replaces a scan of the whole map.
    pub fn any_key_with_prefix(&self, prefix: &str) -> bool {
        let Some(outer) = Self::base_of(prefix) else {
            return self.map.keys().any(|k| k.starts_with(prefix))
                || self
                    .runs
                    .iter()
                    .any(|(name, run)| !run.items.is_empty() && name.starts_with(prefix));
        };
        self.map.keys_with_prefix(outer, prefix).next().is_some()
            || self
                .runs_under(outer)
                .any(|(name, run)| Self::run_has_key_with_prefix(name, run, prefix))
    }

    /// Whether a key `name[i]` of the non-empty `run` begins with `prefix`.
    fn run_has_key_with_prefix(name: &str, run: &ElemRun, prefix: &str) -> bool {
        if prefix.len() <= name.len() {
            return name.starts_with(prefix);
        }
        let Some(rest) = prefix.strip_prefix(name).and_then(|r| r.strip_prefix('[')) else {
            return false;
        };
        // `rest` has to begin `<index>]`, the whole remainder of a key.
        let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        match &rest[digits..] {
            "" => (run.start..run.end()).any(|i| i.to_string().starts_with(rest)),
            "]" => split_elem_key(prefix).is_some_and(|(_, i)| run.slot(i).is_some()),
            _ => false,
        }
    }

    /// Whether `name`'s run holds every element `name[lo]` .. `name[hi - 1]`
    /// and nothing else is stored under `name[`: no element outside the run,
    /// and no member, row or size of an element (`name[i].f`, `name[i][j]`,
    /// `name[i].size`). Then the collection's elements from `lo` on are
    /// exactly the run's items, in order.
    pub fn run_covers(&self, name: &str, lo: u64, hi: u64) -> bool {
        let Some(run) = self.runs.get(name) else {
            return false;
        };
        if run.strays != 0 || run.items.is_empty() || run.start != lo || run.end() < hi {
            return false;
        }
        let outer = Self::outer_of(name);
        let below = |k: &str| {
            k.len() > name.len() && k.as_bytes()[name.len()] == b'[' && k.starts_with(name)
        };
        let has_sparse = if name == outer {
            self.map.indexed.contains_key(outer)
        } else {
            self.map
                .keys_with_prefix(outer, &format!("{name}["))
                .next()
                .is_some()
        };
        !has_sparse
            && !self
                .nested_runs
                .get(outer)
                .is_some_and(|names| names.iter().any(|n| below(n)))
    }

    /// Remove element `name[idx]` of a collection of `size` elements whose
    /// run starts at `lo` (`run_covers(name, lo, size)` holds): the elements
    /// above it move down one index. Elements a shrink left behind at or past
    /// `size` are dropped.
    pub fn run_remove_at(&mut self, name: &str, lo: u64, idx: u64, size: u64) -> Option<Value> {
        note_store_write(signal_key_bits(name));
        let run = self.runs.get_mut(name)?;
        debug_assert!(run.start == lo && lo <= idx && idx < size && run.end() >= size);
        let live = (size - lo) as usize;
        let before = run.items.len();
        run.items.truncate(live);
        let v = if idx == lo {
            run.items.pop_front()
        } else {
            run.items.remove((idx - lo) as usize)
        };
        self.run_keys -= before - run.items.len();
        self.stamp_base(Self::outer_of(name));
        v
    }

    /// Open a slot at `name[idx]` in a collection of `size` elements whose
    /// run starts at `lo` (`run_covers(name, lo, size)` holds): the elements
    /// from `idx` on move up one index, and `name[idx]` keeps its value until
    /// it is stored. Elements a shrink left behind at or past `size` are
    /// dropped.
    pub fn run_open_at(&mut self, name: &str, lo: u64, idx: u64, size: u64) -> bool {
        note_store_write(signal_key_bits(name));
        let Some(run) = self.runs.get_mut(name) else {
            return false;
        };
        debug_assert!(run.start == lo && lo <= idx && idx < size && run.end() >= size);
        let live = (size - lo) as usize;
        let before = run.items.len();
        run.items.truncate(live);
        let s = (idx - lo) as usize;
        let keep = run.items[s].clone();
        if s == 0 {
            run.items.push_front(keep);
        } else {
            run.items.insert(s, keep);
        }
        self.run_keys = self.run_keys + run.items.len() - before;
        self.stamp_base(Self::outer_of(name));
        true
    }
}

impl Default for SignalMap {
    fn default() -> Self {
        Self::new()
    }
}

/// The single place a whole variable is stored (`Simulator::whole_var_store`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum VarStore {
    /// The innermost call frame.
    Frame,
    /// The table signal with this id.
    Table(usize),
}

/// Empty static used as fallback name for unnamed array-element ids
/// (large 1-D arrays have per-element entries skipped to save memory).
pub(super) static EMPTY_NAME: &str = "";

/// The last `.`-separated segment of a hierarchical name and that segment
/// cut at its first `[` — what `rsplit('.')` / `split('[')` give, in one
/// pass over the bytes.
pub(super) fn leaf_and_base(name: &str) -> (&str, &str) {
    let leaf = match name.bytes().rposition(|b| b == b'.') {
        Some(dot) => &name[dot + 1..],
        None => name,
    };
    let base = match leaf.bytes().position(|b| b == b'[') {
        Some(br) => &leaf[..br],
        None => leaf,
    };
    (leaf, base)
}

/// Exact membership prefilter for a string set: a bitmap over (length,
/// last byte). A string whose shape no member has is certainly absent; one
/// that matches a shape still needs the real lookup.
pub(super) struct ShapeFilter {
    bits: Vec<u64>,
}

impl ShapeFilter {
    const LENS: usize = 1024;

    pub(super) fn key(s: &str) -> usize {
        let len = s.len().min(Self::LENS - 1);
        let last = s.as_bytes().last().copied().unwrap_or(0) as usize;
        len * 256 + last
    }

    pub(super) fn new<'a>(members: impl Iterator<Item = &'a str>) -> Self {
        let mut bits = vec![0u64; Self::LENS * 256 / 64];
        for m in members {
            let k = Self::key(m);
            bits[k >> 6] |= 1u64 << (k & 63);
        }
        Self { bits }
    }

    pub(super) fn may_contain(&self, s: &str) -> bool {
        let k = Self::key(s);
        self.bits[k >> 6] & (1u64 << (k & 63)) != 0
    }
}

#[cfg(test)]
mod name_table_helper_tests {
    use super::{ShapeFilter, leaf_and_base};

    #[test]
    fn leaf_and_base_match_the_split_forms() {
        for name in [
            "",
            "a",
            "a.b",
            "top.u1.mem[3]",
            "x[2].y",
            "a.b[1][2]",
            "a..b",
            "a.",
            ".a",
            "[3]",
        ] {
            let leaf = name.rsplit('.').next().unwrap_or(name);
            let base = leaf.split('[').next().unwrap_or(leaf);
            assert_eq!(leaf_and_base(name), (leaf, base), "{name:?}");
        }
    }

    #[test]
    fn shape_filter_never_rejects_a_member() {
        let long = "x".repeat(2000);
        let members = ["a", "top.sig", "mem[3]", "", long.as_str()];
        let f = ShapeFilter::new(members.iter().copied());
        for m in members {
            assert!(f.may_contain(m), "{m:?}");
        }
        assert!(f.may_contain(&"y".repeat(1500).replace('y', "x")));
        assert!(!f.may_contain("b"));
        assert!(!f.may_contain("top.sig2"));
    }
}
