//! §18.5 — joint constraint solving for `randomize()`.
//!
//! The propagation solver in `exec_randomize_inner` draws every variable and
//! then repairs one constraint at a time. Constraints that couple many
//! variables at once — an array `sum()` together with `unique` and an
//! ordering chain — defeat it: each repair breaks another constraint, and
//! every trial fails although the problem is feasible.
//!
//! This solver treats the constraint set as one problem. Every rand scalar
//! and every element of a rand fixed-size or (already sized) dynamic array is
//! an integer variable with an interval-list domain. Constraints translate to
//! linear relations (`==`, `!=`, `<=` over `sum()`, `+`, `-`, `*` by a
//! constant, casts), set membership (`inside`, `dist`), all-different
//! (`unique`) and conditionals (`->`, `if`/`else`, `||`); anything else is
//! judged by the ordinary evaluator once every variable it reads is fixed.
//! A linear relation is only propagated while no operand can wrap at its SV
//! width and signedness (its "fits"), so bounds reasoning never prunes a
//! value the evaluator would accept. Bounds are propagated to a fixpoint and
//! a depth-first search with random variable tie-breaks and random value
//! choice assigns the variables, so solutions spread over the space. A node
//! budget bounds the work: exhausting the search proves infeasibility,
//! running out of budget gives up and leaves the caller's trials in charge.
use super::*;
use crate::ast::decl::DistWeight;
use crate::compiler::elaborate::{is_type_real, is_type_signed, resolve_type_width};
use rand::Rng;

/// Sorted, disjoint, inclusive integer intervals.
type Dom = Vec<(i128, i128)>;

/// Search budget: decisions plus backtracks, over every restart.
const NODE_BUDGET: u64 = 40_000;
/// Constraint executions allowed in one solve (propagation work).
const WORK_BUDGET: u64 = 1_000_000;
/// Largest variable count the solver takes on.
const MAX_VARS: usize = 8192;

fn dom_size(d: &Dom) -> u128 {
    d.iter()
        .fold(0u128, |a, (l, h)| a.saturating_add((h - l) as u128 + 1))
}

fn dom_fixed(d: &Dom) -> Option<i128> {
    (d.len() == 1 && d[0].0 == d[0].1).then(|| d[0].0)
}

fn dom_norm(mut v: Vec<(i128, i128)>) -> Dom {
    v.retain(|(l, h)| l <= h);
    v.sort_unstable();
    let mut out: Dom = Vec::with_capacity(v.len());
    for (l, h) in v {
        if let Some(last) = out.last_mut() {
            if l <= last.1.saturating_add(1) {
                last.1 = last.1.max(h);
                continue;
            }
        }
        out.push((l, h));
    }
    out
}

fn dom_clip(d: &Dom, lo: i128, hi: i128) -> Dom {
    d.iter()
        .filter_map(|&(a, b)| {
            let (a, b) = (a.max(lo), b.min(hi));
            (a <= b).then_some((a, b))
        })
        .collect()
}

fn dom_meet(a: &Dom, b: &Dom) -> Dom {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if lo <= hi {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn dom_minus(a: &Dom, b: &Dom) -> Dom {
    let mut out = Vec::new();
    let mut j = 0;
    for &(mut lo, hi) in a {
        while j < b.len() && b[j].1 < lo {
            j += 1;
        }
        let mut k = j;
        while lo <= hi {
            if k >= b.len() || b[k].0 > hi {
                out.push((lo, hi));
                break;
            }
            if b[k].0 > lo {
                out.push((lo, b[k].0 - 1));
            }
            if b[k].1 >= hi {
                break;
            }
            lo = b[k].1 + 1;
            k += 1;
        }
    }
    out
}

fn dom_has(d: &Dom, v: i128) -> bool {
    d.iter().any(|&(l, h)| l <= v && v <= h)
}

fn dom_nth(d: &Dom, mut k: u128) -> i128 {
    for &(l, h) in d {
        let n = (h - l) as u128 + 1;
        if k < n {
            return l + k as i128;
        }
        k -= n;
    }
    d.last().map_or(0, |x| x.1)
}

fn floor_div(a: i128, b: i128) -> i128 {
    let q = a / b;
    if a % b != 0 && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

fn ceil_div(a: i128, b: i128) -> i128 {
    -floor_div(-a, b)
}

/// The value range of a `w`-bit operand of the given signedness.
fn ws_range(w: u32, s: bool) -> (i128, i128) {
    let w = w.clamp(1, 64);
    if s {
        (-(1i128 << (w - 1)), (1i128 << (w - 1)) - 1)
    } else {
        (0, (1i128 << w) - 1)
    }
}

/// `k + Σ c·x` over solver variables.
#[derive(Clone, Debug, Default)]
struct Lin {
    t: Vec<(usize, i128)>,
    k: i128,
}

/// Coefficients stay far below i128 overflow for 64-bit domains.
const COEF_MAX: i128 = 1 << 40;
const KONST_MAX: i128 = 1 << 100;

impl Lin {
    fn konst(k: i128) -> Self {
        Lin { t: Vec::new(), k }
    }
    fn var(v: usize) -> Self {
        Lin {
            t: vec![(v, 1)],
            k: 0,
        }
    }
    fn is_const(&self) -> bool {
        self.t.is_empty()
    }
    fn plus(mut self, o: &Lin, sgn: i128) -> Option<Lin> {
        self.k = self.k.checked_add(o.k.checked_mul(sgn)?)?;
        self.t.extend(o.t.iter().map(|&(v, c)| (v, c * sgn)));
        self.norm()
    }
    fn scale(mut self, c: i128) -> Option<Lin> {
        self.k = self.k.checked_mul(c)?;
        for t in &mut self.t {
            t.1 = t.1.checked_mul(c)?;
        }
        self.norm()
    }
    fn shift(mut self, d: i128) -> Option<Lin> {
        self.k = self.k.checked_add(d)?;
        (self.k.abs() <= KONST_MAX).then_some(self)
    }
    fn norm(mut self) -> Option<Lin> {
        self.t.sort_unstable_by_key(|t| t.0);
        let mut out: Vec<(usize, i128)> = Vec::with_capacity(self.t.len());
        for (v, c) in self.t {
            match out.last_mut() {
                Some(l) if l.0 == v => l.1 += c,
                _ => out.push((v, c)),
            }
        }
        out.retain(|t| t.1 != 0);
        if self.k.abs() > KONST_MAX || out.iter().any(|t| t.1.abs() > COEF_MAX) {
            return None;
        }
        self.t = out;
        Some(self)
    }
}

/// A linear form that must stay inside `[lo, hi]` for the linear reading of
/// a constraint to equal its SV value (no operand wraps or re-signs).
#[derive(Clone, Debug)]
struct Fit {
    lin: Lin,
    lo: i128,
    hi: i128,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Rel {
    /// `lin <= 0`
    Le,
    /// `lin == 0`
    Eq,
    /// `lin != 0`
    Ne,
}

#[derive(Clone, Debug)]
enum Node {
    True,
    False,
    Lin {
        lin: Lin,
        rel: Rel,
        fits: Vec<Fit>,
        src: usize,
        sneg: bool,
    },
    /// `lin inside set` (or not, with `neg`).
    In {
        lin: Lin,
        set: Dom,
        neg: bool,
        fits: Vec<Fit>,
        src: usize,
    },
    AllDiff(Vec<usize>),
    /// Judged by the evaluator once every variable of `src` is fixed.
    Eval {
        src: usize,
        neg: bool,
    },
    And(Vec<Node>),
    Or(Vec<Node>),
    If {
        cond: Box<Node>,
        ncond: Box<Node>,
        then: Box<Node>,
        els: Box<Node>,
    },
}

#[derive(Clone, Debug)]
enum VarKey {
    Prop(String),
    Elem(String),
}

#[derive(Clone, Debug)]
struct CspVar {
    key: VarKey,
    width: u32,
    signed: bool,
}

#[derive(Clone, Debug)]
enum SrcItem {
    Expr(Expression),
    Item(ConstraintItem),
}

/// The original constraint behind a node, for exact evaluation.
#[derive(Clone, Debug)]
struct Src {
    item: SrcItem,
    binds: Vec<(String, Value)>,
    deps: Vec<usize>,
}

#[derive(Clone, Debug)]
struct CspArr {
    /// (index value, variable) in index order.
    elems: Vec<(i64, usize)>,
    width: u32,
    signed: bool,
}

/// The translated problem.
struct Csp {
    handle: usize,
    vars: Vec<CspVar>,
    dom0: Vec<Dom>,
    scalars: HashMap<String, usize>,
    arrays: HashMap<String, CspArr>,
    srcs: Vec<Src>,
    nodes: Vec<Node>,
    /// Whether any `soft` item was seen (translation with `with_soft`).
    has_soft: bool,
    with_soft: bool,
}

/// Translation scope: bound `foreach` indices and the `with` iterator.
#[derive(Clone, Default)]
struct Env {
    binds: Vec<(String, Value)>,
    /// (iterator name, element variable, element index)
    it: Option<(String, usize, i64)>,
}

/// An arithmetic operand before its context is known (§11.8.2).
#[derive(Clone, Debug)]
enum Ae {
    /// (variable, width, signed)
    Var(usize, u32, bool),
    Const(Value),
    Neg(Box<Ae>),
    Add(Box<Ae>, Box<Ae>),
    Sub(Box<Ae>, Box<Ae>),
    Mul(Box<Ae>, Box<Ae>),
    /// Array reduction accumulated at its own `(w, s)` (§7.12.3).
    Sum {
        terms: Vec<Ae>,
        w: u32,
        s: bool,
    },
    /// Cast: `inner` evaluated in the context `(iw, is)` — never narrower
    /// than the target (§6.24.1) — then converted to `(w, s)`.
    Cast {
        inner: Box<Ae>,
        iw: u32,
        is: bool,
        w: u32,
        s: bool,
    },
}

/// Search state: current domains plus an undo trail.
struct St {
    d: Vec<Dom>,
    trail: Vec<(usize, Dom)>,
    dirty: Vec<usize>,
    work: u64,
}

impl St {
    /// Replace a domain; false when it became empty.
    fn set(&mut self, v: usize, nd: Dom) -> bool {
        if nd == self.d[v] {
            return true;
        }
        let empty = nd.is_empty();
        let old = std::mem::replace(&mut self.d[v], nd);
        self.trail.push((v, old));
        self.dirty.push(v);
        !empty
    }
    fn undo(&mut self, mark: usize) {
        while self.trail.len() > mark {
            let (v, old) = self.trail.pop().unwrap();
            self.d[v] = old;
        }
    }
    fn bounds(&self, v: usize) -> (i128, i128) {
        let d = &self.d[v];
        (d.first().map_or(0, |x| x.0), d.last().map_or(-1, |x| x.1))
    }
    fn fixed(&self, v: usize) -> Option<i128> {
        dom_fixed(&self.d[v])
    }
    fn lin_bounds(&self, l: &Lin) -> (i128, i128) {
        let (mut lo, mut hi) = (l.k, l.k);
        for &(v, c) in &l.t {
            let (a, b) = self.bounds(v);
            if c > 0 {
                lo += c * a;
                hi += c * b;
            } else {
                lo += c * b;
                hi += c * a;
            }
        }
        (lo, hi)
    }
    fn fits_hold(&self, fits: &[Fit]) -> bool {
        fits.iter().all(|f| {
            let (lo, hi) = self.lin_bounds(&f.lin);
            lo >= f.lo && hi <= f.hi
        })
    }
    fn all_fixed(&self, vs: &[usize]) -> bool {
        vs.iter().all(|&v| self.fixed(v).is_some())
    }
    /// `lin <= 0` (upper) or `lin >= 0` (lower) by bounds.
    fn prop_bound(&mut self, l: &Lin, upper: bool) -> bool {
        let (lo, hi) = self.lin_bounds(l);
        if (upper && lo > 0) || (!upper && hi < 0) {
            return false;
        }
        for &(v, c) in &l.t {
            let (a, b) = self.bounds(v);
            let (tmin, tmax) = if c > 0 {
                (c * a, c * b)
            } else {
                (c * b, c * a)
            };
            let d = &self.d[v];
            let nd = if upper {
                // c·x <= tmin - lo
                let r = tmin - lo;
                if c > 0 {
                    let h = floor_div(r, c);
                    if h >= b {
                        continue;
                    }
                    dom_clip(d, i128::MIN, h)
                } else {
                    let l2 = ceil_div(r, c);
                    if l2 <= a {
                        continue;
                    }
                    dom_clip(d, l2, i128::MAX)
                }
            } else {
                // c·x >= tmax - hi
                let r = tmax - hi;
                if c > 0 {
                    let l2 = ceil_div(r, c);
                    if l2 <= a {
                        continue;
                    }
                    dom_clip(d, l2, i128::MAX)
                } else {
                    let h = floor_div(r, c);
                    if h >= b {
                        continue;
                    }
                    dom_clip(d, i128::MIN, h)
                }
            };
            if !self.set(v, nd) {
                return false;
            }
        }
        true
    }
}

impl Node {
    fn deps(&self, srcs: &[Src], out: &mut Vec<usize>) {
        match self {
            Node::True | Node::False => {}
            Node::Lin { lin, fits, src, .. } | Node::In { lin, fits, src, .. } => {
                out.extend(lin.t.iter().map(|t| t.0));
                for f in fits {
                    out.extend(f.lin.t.iter().map(|t| t.0));
                }
                out.extend(srcs[*src].deps.iter().copied());
            }
            Node::AllDiff(vs) => out.extend(vs.iter().copied()),
            Node::Eval { src, .. } => out.extend(srcs[*src].deps.iter().copied()),
            Node::And(ns) | Node::Or(ns) => ns.iter().for_each(|n| n.deps(srcs, out)),
            Node::If {
                cond, then, els, ..
            } => {
                cond.deps(srcs, out);
                then.deps(srcs, out);
                els.deps(srcs, out);
            }
        }
    }
}

/// Outcome of one joint solve.
pub(super) enum CspOutcome {
    /// A verified solution was written to the object.
    Sat,
    /// The search space was exhausted: no solution exists.
    Unsat,
    /// Budget exhausted (or the verifier disagreed); nothing is known.
    GaveUp,
    /// The constraint set uses something this solver does not model.
    NotApplicable,
}

impl Simulator {
    /// Solve the constraint set of `handle` jointly (see the module docs).
    /// `rand_props` are the scalar rand properties, `colls` the rand
    /// collections (dynamic ones already sized), `array_enums` the enum type
    /// of enum-typed fixed-array elements.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rand_csp_solve(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
    ) -> CspOutcome {
        let Some(mut csp) = self.csp_vars(
            handle,
            rand_props,
            signed_props,
            enum_props,
            colls,
            array_enums,
        ) else {
            return CspOutcome::NotApplicable;
        };
        csp.with_soft = true;
        if self.csp_translate(&mut csp, constraints).is_none() {
            return CspOutcome::NotApplicable;
        }
        let mut out = self.csp_run(&csp, constraints, colls);
        if !matches!(out, CspOutcome::Sat) && csp.has_soft {
            // §18.5.14: soft constraints yield when the hard set cannot
            // hold together with them.
            csp.with_soft = false;
            csp.srcs.clear();
            csp.nodes.clear();
            if self.csp_translate(&mut csp, constraints).is_none() {
                return CspOutcome::NotApplicable;
            }
            out = self.csp_run(&csp, constraints, colls);
        }
        out
    }

    /// Build the variable table: rand scalars and array elements.
    fn csp_vars(
        &mut self,
        handle: usize,
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
    ) -> Option<Csp> {
        let mut csp = Csp {
            handle,
            vars: Vec::new(),
            dom0: Vec::new(),
            scalars: HashMap::default(),
            arrays: HashMap::default(),
            srcs: Vec::new(),
            nodes: Vec::new(),
            has_soft: false,
            with_soft: false,
        };
        let enum_dom = |me: &Self, tn: &str| -> Option<Dom> {
            let members = me.module.enum_members.get(tn)?;
            if members.is_empty() {
                return None;
            }
            Some(dom_norm(
                members.iter().map(|m| (m.1 as i128, m.1 as i128)).collect(),
            ))
        };
        for c in colls {
            if c.is_object_elem || c.nested || c.kind == CollKind::Assoc || c.width > 64 {
                return None;
            }
            let signed = self.class_prop_signed_of(handle, &c.prop);
            let dom = array_enums
                .get(&c.prop)
                .and_then(|tn| enum_dom(self, tn))
                .unwrap_or_else(|| {
                    let (lo, hi) = ws_range(c.width, signed);
                    vec![(lo, hi)]
                });
            let idx: Vec<i64> = match c.kind {
                CollKind::Fixed => (c.lo..=c.hi).collect(),
                _ => (0..self.get_queue_size(&c.scoped) as i64).collect(),
            };
            let mut elems = Vec::with_capacity(idx.len());
            for i in idx {
                elems.push((i, csp.vars.len()));
                csp.vars.push(CspVar {
                    key: VarKey::Elem(format!("{}[{}]", c.scoped, i)),
                    width: c.width,
                    signed,
                });
                csp.dom0.push(dom.clone());
            }
            csp.arrays.insert(
                c.prop.clone(),
                CspArr {
                    elems,
                    width: c.width,
                    signed,
                },
            );
        }
        for (name, w) in rand_props {
            if csp.arrays.contains_key(name) {
                continue;
            }
            if *w == 0 || *w > 64 {
                return None;
            }
            let signed = signed_props.contains(name);
            let dom = enum_props
                .get(name)
                .and_then(|tn| enum_dom(self, tn))
                .unwrap_or_else(|| {
                    let (lo, hi) = ws_range(*w, signed);
                    vec![(lo, hi)]
                });
            csp.scalars.insert(name.clone(), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Prop(name.clone()),
                width: *w,
                signed,
            });
            csp.dom0.push(dom);
        }
        if csp.vars.is_empty() || csp.vars.len() > MAX_VARS {
            return None;
        }
        Some(csp)
    }

    fn csp_translate(&mut self, csp: &mut Csp, constraints: &[ClassConstraint]) -> Option<()> {
        let env = Env::default();
        for con in constraints {
            for it in &con.items {
                let n = self.csp_item(csp, it, &env)?;
                if !matches!(n, Node::True) {
                    csp.nodes.push(n);
                }
            }
        }
        Some(())
    }

    /// Search with restarts, then write and verify a solution.
    fn csp_run(
        &mut self,
        csp: &Csp,
        constraints: &[ClassConstraint],
        colls: &[RandColl],
    ) -> CspOutcome {
        let nvars = csp.vars.len();
        let mut watch: Vec<Vec<usize>> = vec![Vec::new(); nvars];
        for (i, n) in csp.nodes.iter().enumerate() {
            let mut ds = Vec::new();
            n.deps(&csp.srcs, &mut ds);
            ds.sort_unstable();
            ds.dedup();
            for v in ds {
                watch[v].push(i);
            }
        }
        let mut budget = NODE_BUDGET;
        let mut run_budget: u64 = 1000;
        let mut st = St {
            d: csp.dom0.clone(),
            trail: Vec::new(),
            dirty: Vec::new(),
            work: 0,
        };
        loop {
            st.d.clone_from(&csp.dom0);
            st.trail.clear();
            st.dirty.clear();
            let mut run = run_budget.min(budget);
            let r = self.csp_search(csp, &watch, &mut st, &mut run);
            budget -= run_budget.min(budget) - run;
            match r {
                Some(true) => break,
                Some(false) => return CspOutcome::Unsat,
                None if budget == 0 || st.work >= WORK_BUDGET => return CspOutcome::GaveUp,
                None => run_budget *= 2,
            }
        }
        for v in 0..nvars {
            let x = st.fixed(v).unwrap_or(0);
            self.csp_write(csp, v, x);
        }
        if self.rand_items_accept(csp.handle, constraints, colls, &mut false) {
            CspOutcome::Sat
        } else {
            if std::env::var_os("XEZIM_RAND_DBG").is_some() {
                eprintln!("[rand-dbg] joint solve found a solution the checker rejects");
            }
            CspOutcome::GaveUp
        }
    }

    /// Depth-first search: `Some(true)` solved (every domain fixed),
    /// `Some(false)` exhausted, `None` out of budget.
    fn csp_search(
        &mut self,
        csp: &Csp,
        watch: &[Vec<usize>],
        st: &mut St,
        budget: &mut u64,
    ) -> Option<bool> {
        let all: Vec<usize> = (0..csp.nodes.len()).collect();
        match self.csp_propagate(csp, watch, st, &all) {
            Some(true) => {}
            Some(false) => return Some(false),
            None => return None,
        }
        // (variable, value, trail mark) per decision level
        let mut stack: Vec<(usize, i128, usize)> = Vec::new();
        loop {
            let Some(v) = self.csp_pick_var(st) else {
                return Some(true);
            };
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            let size = dom_size(&st.d[v]);
            let k = self.cur_rng().gen_range(0..size);
            let val = dom_nth(&st.d[v], k);
            stack.push((v, val, st.trail.len()));
            st.dirty.clear();
            st.set(v, vec![(val, val)]);
            let dirty = std::mem::take(&mut st.dirty);
            match self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty)) {
                Some(true) => continue,
                Some(false) => {}
                None => return None,
            }
            // Conflict: undo decisions until `x != v` leaves a consistent state.
            loop {
                let Some((v, val, mark)) = stack.pop() else {
                    return Some(false);
                };
                st.undo(mark);
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
                st.dirty.clear();
                let nd = dom_minus(&st.d[v], &vec![(val, val)]);
                if !st.set(v, nd) {
                    continue;
                }
                let dirty = std::mem::take(&mut st.dirty);
                match self.csp_propagate(csp, watch, st, &Self::csp_watchers(watch, &dirty)) {
                    Some(true) => break,
                    Some(false) => continue,
                    None => return None,
                }
            }
        }
    }

    fn csp_watchers(watch: &[Vec<usize>], vars: &[usize]) -> Vec<usize> {
        let mut out: Vec<usize> = vars
            .iter()
            .flat_map(|&v| watch[v].iter().copied())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Smallest unfixed domain, ties broken at random.
    fn csp_pick_var(&mut self, st: &St) -> Option<usize> {
        let mut best: Option<(u128, usize)> = None;
        let mut ties = 0u32;
        for (v, d) in st.d.iter().enumerate() {
            if dom_fixed(d).is_some() {
                continue;
            }
            let n = dom_size(d);
            match best {
                Some((bn, _)) if n > bn => {}
                Some((bn, _)) if n == bn => {
                    ties += 1;
                    if self.cur_rng().gen_range(0..=ties) == 0 {
                        best = Some((n, v));
                    }
                }
                _ => {
                    best = Some((n, v));
                    ties = 0;
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Run the queued nodes to a fixpoint. `Some(false)` on a conflict,
    /// `None` when the work budget is gone.
    fn csp_propagate(
        &mut self,
        csp: &Csp,
        watch: &[Vec<usize>],
        st: &mut St,
        start: &[usize],
    ) -> Option<bool> {
        let n = csp.nodes.len();
        let mut queued = vec![false; n];
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for &i in start {
            if !queued[i] {
                queued[i] = true;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            queued[i] = false;
            st.work += 1;
            if st.work >= WORK_BUDGET {
                return None;
            }
            st.dirty.clear();
            if !self.csp_enforce(csp, &csp.nodes[i], st) {
                return Some(false);
            }
            let dirty = std::mem::take(&mut st.dirty);
            for v in dirty {
                for &j in &watch[v] {
                    if !queued[j] {
                        queued[j] = true;
                        queue.push_back(j);
                    }
                }
            }
        }
        Some(true)
    }

    /// Make `n` hold under the current domains; false on a conflict.
    fn csp_enforce(&mut self, csp: &Csp, n: &Node, st: &mut St) -> bool {
        match n {
            Node::True => true,
            Node::False => false,
            Node::Lin {
                lin,
                rel,
                fits,
                src,
                sneg,
            } => {
                if !st.fits_hold(fits) {
                    return !self.csp_src_fixed(csp, st, *src)
                        || self.csp_eval_src(csp, st, *src) != *sneg;
                }
                match rel {
                    Rel::Le => st.prop_bound(lin, true),
                    Rel::Eq => st.prop_bound(lin, true) && st.prop_bound(lin, false),
                    Rel::Ne => {
                        let mut free: Option<(usize, i128)> = None;
                        let mut rest = lin.k;
                        for &(v, c) in &lin.t {
                            match st.fixed(v) {
                                Some(x) => rest += c * x,
                                None if free.is_none() => free = Some((v, c)),
                                None => return true,
                            }
                        }
                        match free {
                            None => rest != 0,
                            Some((v, c)) => {
                                if (-rest) % c != 0 {
                                    return true;
                                }
                                let x = -rest / c;
                                let nd = dom_minus(&st.d[v], &vec![(x, x)]);
                                st.set(v, nd)
                            }
                        }
                    }
                }
            }
            Node::In {
                lin,
                set,
                neg,
                fits,
                src,
            } => {
                if !st.fits_hold(fits) {
                    return !self.csp_src_fixed(csp, st, *src)
                        || self.csp_eval_src(csp, st, *src) != *neg;
                }
                if let [(v, c)] = lin.t[..] {
                    if c == 1 || c == -1 {
                        let allowed: Dom = if c == 1 {
                            set.iter().map(|&(l, h)| (l - lin.k, h - lin.k)).collect()
                        } else {
                            dom_norm(set.iter().map(|&(l, h)| (lin.k - h, lin.k - l)).collect())
                        };
                        let nd = if *neg {
                            dom_minus(&st.d[v], &allowed)
                        } else {
                            dom_meet(&st.d[v], &allowed)
                        };
                        return st.set(v, nd);
                    }
                }
                let (lo, hi) = st.lin_bounds(lin);
                if lo == hi {
                    return dom_has(set, lo) != *neg;
                }
                if *neg || set.is_empty() {
                    return !set.is_empty() || *neg;
                }
                let (smin, smax) = (set[0].0, set[set.len() - 1].1);
                let (Some(up), Some(dn)) = (lin.clone().shift(-smax), lin.clone().shift(-smin))
                else {
                    return true;
                };
                st.prop_bound(&up, true) && st.prop_bound(&dn, false)
            }
            Node::AllDiff(vs) => {
                let mut taken: Vec<(i128, i128)> = Vec::new();
                for &v in vs {
                    if let Some(x) = st.fixed(v) {
                        taken.push((x, x));
                    }
                }
                let before = taken.len();
                let taken = dom_norm(taken);
                if dom_size(&taken) < before as u128 {
                    return false;
                }
                let mut free = 0u128;
                let mut pool: Vec<(i128, i128)> = Vec::new();
                for &v in vs {
                    if st.fixed(v).is_some() {
                        continue;
                    }
                    let nd = dom_minus(&st.d[v], &taken);
                    if !st.set(v, nd) {
                        return false;
                    }
                    free += 1;
                    pool.extend(st.d[v].iter().copied());
                }
                // pigeonhole: the free variables need that many values
                free == 0 || dom_size(&dom_norm(pool)) >= free
            }
            Node::Eval { src, neg } => {
                !self.csp_src_fixed(csp, st, *src) || self.csp_eval_src(csp, st, *src) != *neg
            }
            Node::And(ns) => ns.iter().all(|m| self.csp_enforce(csp, m, st)),
            Node::Or(ns) => {
                let mut open: Option<&Node> = None;
                let mut n_open = 0;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(true) => return true,
                        Some(false) => {}
                        None => {
                            n_open += 1;
                            open = Some(m);
                        }
                    }
                }
                match n_open {
                    0 => false,
                    1 => self.csp_enforce(csp, open.unwrap(), st),
                    _ => true,
                }
            }
            Node::If {
                cond,
                ncond,
                then,
                els,
            } => match self.csp_status(csp, cond, st) {
                Some(true) => self.csp_enforce(csp, then, st),
                Some(false) => self.csp_enforce(csp, els, st),
                None => {
                    if self.csp_status(csp, then, st) == Some(false) {
                        return self.csp_enforce(csp, ncond, st);
                    }
                    if self.csp_status(csp, els, st) == Some(false) {
                        return self.csp_enforce(csp, cond, st);
                    }
                    true
                }
            },
        }
    }

    /// Entailed (`Some(true)`), refuted (`Some(false)`) or open.
    fn csp_status(&mut self, csp: &Csp, n: &Node, st: &St) -> Option<bool> {
        match n {
            Node::True => Some(true),
            Node::False => Some(false),
            Node::Lin {
                lin,
                rel,
                fits,
                src,
                sneg,
            } => {
                if !st.fits_hold(fits) {
                    if self.csp_src_fixed(csp, st, *src) {
                        return Some(self.csp_eval_src(csp, st, *src) != *sneg);
                    }
                    return None;
                }
                let (lo, hi) = st.lin_bounds(lin);
                let eq = if lo == 0 && hi == 0 {
                    Some(true)
                } else if lo > 0 || hi < 0 {
                    Some(false)
                } else {
                    None
                };
                match rel {
                    Rel::Le if hi <= 0 => Some(true),
                    Rel::Le if lo > 0 => Some(false),
                    Rel::Le => None,
                    Rel::Eq => eq,
                    Rel::Ne => eq.map(|b| !b),
                }
            }
            Node::In {
                lin,
                set,
                neg,
                fits,
                src,
            } => {
                if !st.fits_hold(fits) {
                    if self.csp_src_fixed(csp, st, *src) {
                        return Some(self.csp_eval_src(csp, st, *src) != *neg);
                    }
                    return None;
                }
                let (lo, hi) = st.lin_bounds(lin);
                if lo == hi {
                    return Some(dom_has(set, lo) != *neg);
                }
                if dom_clip(set, lo, hi).is_empty() {
                    return Some(*neg);
                }
                if set.iter().any(|&(l, h)| l <= lo && hi <= h) {
                    return Some(!*neg);
                }
                None
            }
            Node::AllDiff(vs) => {
                if !st.all_fixed(vs) {
                    return None;
                }
                let mut xs: Vec<i128> = vs.iter().map(|&v| st.fixed(v).unwrap()).collect();
                xs.sort_unstable();
                Some(xs.windows(2).all(|w| w[0] != w[1]))
            }
            Node::Eval { src, neg } => {
                if self.csp_src_fixed(csp, st, *src) {
                    Some(self.csp_eval_src(csp, st, *src) != *neg)
                } else {
                    None
                }
            }
            Node::And(ns) => {
                let mut all = true;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(false) => return Some(false),
                        Some(true) => {}
                        None => all = false,
                    }
                }
                all.then_some(true)
            }
            Node::Or(ns) => {
                let mut none = true;
                for m in ns {
                    match self.csp_status(csp, m, st) {
                        Some(true) => return Some(true),
                        Some(false) => {}
                        None => none = false,
                    }
                }
                none.then_some(false)
            }
            Node::If {
                cond, then, els, ..
            } => match self.csp_status(csp, cond, st) {
                Some(true) => self.csp_status(csp, then, st),
                Some(false) => self.csp_status(csp, els, st),
                None => {
                    let (t, e) = (
                        self.csp_status(csp, then, st),
                        self.csp_status(csp, els, st),
                    );
                    if t.is_some() && t == e { t } else { None }
                }
            },
        }
    }

    fn csp_src_fixed(&self, csp: &Csp, st: &St, src: usize) -> bool {
        st.all_fixed(&csp.srcs[src].deps)
    }

    /// Judge a source constraint with the ordinary evaluator: the fixed
    /// variables it reads are written to the object first.
    fn csp_eval_src(&mut self, csp: &Csp, st: &St, src: usize) -> bool {
        let s = &csp.srcs[src];
        for &v in &s.deps {
            if let Some(x) = st.fixed(v) {
                self.csp_write(csp, v, x);
            }
        }
        let frame: HashMap<String, Value> = s.binds.iter().cloned().collect();
        self.push_local_frame(frame);
        let ok = match &s.item {
            SrcItem::Expr(e) => self.cons_expr_true(e),
            SrcItem::Item(it) => self.check_constraint_item_impl(it),
        };
        self.pop_local_frame();
        ok
    }

    fn csp_write(&mut self, csp: &Csp, v: usize, x: i128) {
        let var = &csp.vars[v];
        let mut val = Value::from_u64(Self::bits_at(x, var.width) as u64, var.width);
        val.is_signed = var.signed;
        match &var.key {
            VarKey::Prop(n) => {
                if let Some(Some(inst)) = self.heap.get_mut(csp.handle) {
                    inst.properties.insert(n.clone(), val);
                }
            }
            VarKey::Elem(k) => self.write_coll_elem(k, val),
        }
    }

    // ---------------------------------------------------------------------
    // Translation
    // ---------------------------------------------------------------------

    fn csp_src(&self, csp: &mut Csp, item: SrcItem, env: &Env, deps: Vec<usize>) -> usize {
        csp.srcs.push(Src {
            item,
            binds: env.binds.clone(),
            deps,
        });
        csp.srcs.len() - 1
    }

    /// A node judged by the evaluator alone. None when its variables cannot
    /// be determined.
    fn csp_eval_node(
        &mut self,
        csp: &mut Csp,
        item: SrcItem,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let mut deps = Vec::new();
        let complete = match &item {
            SrcItem::Expr(e) => self.csp_refs(csp, e, env, &mut deps),
            SrcItem::Item(it) => self.csp_item_refs(csp, it, env, &mut deps),
        };
        if !complete {
            // An opaque call may read any rand member.
            deps = (0..csp.vars.len()).collect();
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, item, env, deps);
        Some(Node::Eval { src, neg })
    }

    fn csp_item(&mut self, csp: &mut Csp, item: &ConstraintItem, env: &Env) -> Option<Node> {
        match item {
            ConstraintItem::Expr(e) => self.csp_bool(csp, e, env, false),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                let range: Vec<ConstraintRange> = if *is_dist {
                    // §18.5.4: a zero weight removes the value.
                    let mut kept = Vec::new();
                    for (k, r) in range.iter().enumerate() {
                        let w = match dist_weights.get(k).and_then(|w| w.as_ref()) {
                            Some(DistWeight::Each(e)) | Some(DistWeight::Total(e)) => {
                                self.csp_const(e, env).and_then(|v| v.to_u64())
                            }
                            None => Some(1),
                        };
                        if w != Some(0) {
                            kept.push(r.clone());
                        }
                    }
                    kept
                } else {
                    range.clone()
                };
                self.csp_inside(csp, expr, &range, env, false, SrcItem::Item(item.clone()))
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                let then = self.csp_item(csp, constraint, env)?;
                self.csp_if(csp, condition, then, Node::True, env)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                let then = self.csp_item(csp, then_item, env)?;
                let els = match else_item {
                    Some(e) => self.csp_item(csp, e, env)?,
                    None => Node::True,
                };
                self.csp_if(csp, condition, then, els, env)
            }
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                let name = Self::foreach_base_name(array);
                let arr = name.as_ref().and_then(|n| csp.arrays.get(n)).cloned();
                let idx_var = match vars.as_slice() {
                    [Some(v)] => Some(v.name.clone()),
                    _ => None,
                };
                let (Some(arr), Some(iv)) = (arr, idx_var) else {
                    return self.csp_eval_node(csp, SrcItem::Item(item.clone()), env, false);
                };
                let mut out = Vec::with_capacity(arr.elems.len());
                for (i, _) in &arr.elems {
                    let mut e2 = env.clone();
                    // bound the way the checker binds a foreach index
                    e2.binds.push((iv.clone(), Value::from_u64(*i as u64, 32)));
                    let n = self.csp_item(csp, body, &e2)?;
                    match n {
                        Node::True => {}
                        Node::False => return Some(Node::False),
                        n => out.push(n),
                    }
                }
                Some(Self::csp_and(out))
            }
            ConstraintItem::Solve { .. } => Some(Node::True),
            ConstraintItem::Soft(inner) => {
                csp.has_soft = true;
                if csp.with_soft {
                    self.csp_item(csp, inner, env)
                } else {
                    Some(Node::True)
                }
            }
            ConstraintItem::Block(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match self.csp_item(csp, it, env)? {
                        Node::True => {}
                        Node::False => return Some(Node::False),
                        n => out.push(n),
                    }
                }
                Some(Self::csp_and(out))
            }
            ConstraintItem::Unique { exprs, .. } => {
                let mut vs = Vec::new();
                for e in exprs {
                    let e = Self::unparen(e);
                    if let Some(a) = self.csp_member(csp, e).and_then(|n| csp.arrays.get(&n)) {
                        vs.extend(a.elems.iter().map(|x| x.1));
                        continue;
                    }
                    match self.csp_ae(csp, e, env) {
                        Some(Ae::Var(v, ..)) => vs.push(v),
                        _ => {
                            return self.csp_eval_node(
                                csp,
                                SrcItem::Item(item.clone()),
                                env,
                                false,
                            );
                        }
                    }
                }
                Some(Node::AllDiff(vs))
            }
        }
    }

    fn csp_and(mut v: Vec<Node>) -> Node {
        match v.len() {
            0 => Node::True,
            1 => v.pop().unwrap(),
            _ => Node::And(v),
        }
    }

    fn csp_if(
        &mut self,
        csp: &mut Csp,
        cond: &Expression,
        then: Node,
        els: Node,
        env: &Env,
    ) -> Option<Node> {
        let c = self.csp_bool(csp, cond, env, false)?;
        match c {
            Node::True => return Some(then),
            Node::False => return Some(els),
            _ => {}
        }
        if matches!(then, Node::True) && matches!(els, Node::True) {
            return Some(Node::True);
        }
        let nc = self.csp_bool(csp, cond, env, true)?;
        Some(Node::If {
            cond: Box::new(c),
            ncond: Box::new(nc),
            then: Box::new(then),
            els: Box::new(els),
        })
    }

    /// A boolean constraint expression (negated with `neg`).
    fn csp_bool(&mut self, csp: &mut Csp, e: &Expression, env: &Env, neg: bool) -> Option<Node> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            let v = self.csp_const_any(e, env);
            return Some(if v.is_true() != neg {
                Node::True
            } else {
                Node::False
            });
        }
        match &e.kind {
            ExprKind::Unary {
                op: UnaryOp::LogNot,
                operand,
            } => self.csp_bool(csp, operand, env, !neg),
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::LogAnd | BinaryOp::LogOr => {
                    let l = self.csp_bool(csp, left, env, neg)?;
                    let r = self.csp_bool(csp, right, env, neg)?;
                    // De Morgan: a negated && is an || of the negations
                    Some(if (*op == BinaryOp::LogAnd) != neg {
                        Node::And(vec![l, r])
                    } else {
                        Node::Or(vec![l, r])
                    })
                }
                BinaryOp::LogImplies => {
                    if neg {
                        let l = self.csp_bool(csp, left, env, false)?;
                        let r = self.csp_bool(csp, right, env, true)?;
                        Some(Node::And(vec![l, r]))
                    } else {
                        let then = self.csp_bool(csp, right, env, false)?;
                        self.csp_if(csp, left, then, Node::True, env)
                    }
                }
                BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::CaseEq
                | BinaryOp::CaseNeq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq => self.csp_rel(csp, e, *op, left, right, env, neg),
                _ => self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg),
            },
            ExprKind::Inside { expr, ranges } => {
                let cr: Vec<ConstraintRange> = ranges
                    .iter()
                    .map(|r| match &r.kind {
                        ExprKind::Range(lo, hi) => ConstraintRange::Range {
                            lo: (**lo).clone(),
                            hi: (**hi).clone(),
                        },
                        _ => ConstraintRange::Value(r.clone()),
                    })
                    .collect();
                self.csp_inside(csp, expr, &cr, env, neg, SrcItem::Expr(e.clone()))
            }
            _ => self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg),
        }
    }

    /// `left op right` (§11.6.1: both sides extended to the context width
    /// and signedness).
    #[allow(clippy::too_many_arguments)]
    fn csp_rel(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let fallback = |me: &mut Self, csp: &mut Csp| {
            me.csp_eval_node(csp, SrcItem::Expr(whole.clone()), env, neg)
        };
        let (Some(a), Some(b)) = (self.csp_ae(csp, left, env), self.csp_ae(csp, right, env)) else {
            return fallback(self, csp);
        };
        let ((wl, sl), (wr, sr)) = (Self::csp_ws(&a), Self::csp_ws(&b));
        let w = wl.max(wr).max(1);
        if w > 64 {
            return fallback(self, csp);
        }
        let s = sl && sr;
        let mut fits = Vec::new();
        let (Some(la), Some(lb)) = (
            Self::csp_lin(&a, w, s, &mut fits),
            Self::csp_lin(&b, w, s, &mut fits),
        ) else {
            return fallback(self, csp);
        };
        let (rlo, rhi) = ws_range(w, s);
        fits.push(Fit {
            lin: la.clone(),
            lo: rlo,
            hi: rhi,
        });
        fits.push(Fit {
            lin: lb.clone(),
            lo: rlo,
            hi: rhi,
        });
        let Some(d) = la.plus(&lb, -1) else {
            return fallback(self, csp);
        };
        let op = if neg {
            match op {
                BinaryOp::Eq | BinaryOp::CaseEq => BinaryOp::Neq,
                BinaryOp::Neq | BinaryOp::CaseNeq => BinaryOp::Eq,
                BinaryOp::Lt => BinaryOp::Geq,
                BinaryOp::Leq => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Leq,
                _ => BinaryOp::Lt,
            }
        } else {
            op
        };
        let (lin, rel) = match op {
            BinaryOp::Eq | BinaryOp::CaseEq => (Some(d), Rel::Eq),
            BinaryOp::Neq | BinaryOp::CaseNeq => (Some(d), Rel::Ne),
            BinaryOp::Lt => (d.shift(1), Rel::Le),
            BinaryOp::Leq => (Some(d), Rel::Le),
            BinaryOp::Gt => (d.scale(-1).and_then(|x| x.shift(1)), Rel::Le),
            _ => (d.scale(-1), Rel::Le),
        };
        let Some(lin) = lin else {
            return fallback(self, csp);
        };
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        Some(Node::Lin {
            lin,
            rel,
            fits,
            src,
            sneg: neg,
        })
    }

    /// `expr inside {ranges}` with constant ranges; judged like the checker
    /// judges it (`value_in_ranges` on the operand's own value).
    #[allow(clippy::too_many_arguments)]
    fn csp_inside(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        env: &Env,
        neg: bool,
        whole: SrcItem,
    ) -> Option<Node> {
        let ranges_free = ranges.iter().all(|r| match r {
            ConstraintRange::Value(e) => self.csp_free(csp, e, env),
            ConstraintRange::Range { lo, hi } => {
                self.csp_free(csp, lo, env) && self.csp_free(csp, hi, env)
            }
        });
        let a = if ranges_free {
            self.csp_ae(csp, expr, env)
        } else {
            None
        };
        let Some(a) = a else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        let (w, s) = Self::csp_ws(&a);
        let mut fits = Vec::new();
        let lin = if w <= 64 {
            Self::csp_lin(&a, w, s, &mut fits)
        } else {
            None
        };
        let Some(lin) = lin else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        let frame: HashMap<String, Value> = env.binds.iter().cloned().collect();
        self.push_local_frame(frame);
        let set = dom_norm(self.cons_ranges_i128(ranges));
        self.pop_local_frame();
        // A negative endpoint makes the checker read an unsigned value as
        // signed; leave that reading to the checker itself.
        if !s && set.iter().any(|r| r.0 < 0) {
            return self.csp_eval_node(csp, whole, env, neg);
        }
        let (rlo, rhi) = ws_range(w, s);
        fits.push(Fit {
            lin: lin.clone(),
            lo: rlo,
            hi: rhi,
        });
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, whole, env, deps);
        Some(Node::In {
            lin,
            set,
            neg,
            fits,
            src,
        })
    }

    /// The object's own member named by `e` (`x`, `this.x`, `obj.x` for the
    /// inline-constraint receiver), if it is a solver scalar or array.
    fn csp_member(&self, csp: &Csp, e: &Expression) -> Option<String> {
        let known = |n: &str| csp.scalars.contains_key(n) || csp.arrays.contains_key(n);
        let recv_ok = |me: &Self, r: &str| r == "this" || me.rand_receiver.as_deref() == Some(r);
        let name = match &e.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => match h.path.len()
            {
                1 => h.path[0].name.name.clone(),
                2 if recv_ok(self, &h.path[0].name.name) => h.path[1].name.name.clone(),
                _ => return None,
            },
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::This => member.name.clone(),
                ExprKind::Ident(h) if h.path.len() == 1 && recv_ok(self, &h.path[0].name.name) => {
                    member.name.clone()
                }
                _ => return None,
            },
            _ => return None,
        };
        known(&name).then_some(name)
    }

    /// Variables `e` reads. False when it calls something whose reads are
    /// unknown (a user function).
    fn csp_refs(&self, csp: &Csp, e: &Expression, env: &Env, out: &mut Vec<usize>) -> bool {
        let push_name = |n: &str, out: &mut Vec<usize>| {
            if let Some(&v) = csp.scalars.get(n) {
                out.push(v);
            } else if let Some(a) = csp.arrays.get(n) {
                out.extend(a.elems.iter().map(|x| x.1));
            }
        };
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.path.len() == 1 {
                    let n = &h.path[0].name.name;
                    if env.binds.iter().any(|b| &b.0 == n) {
                        return true;
                    }
                    if let Some((it, v, _)) = &env.it {
                        if it == n {
                            out.push(*v);
                            return true;
                        }
                    }
                }
                if let Some(n) = self.csp_member(csp, e) {
                    push_name(&n, out);
                } else if let Some(seg) = h.path.last() {
                    // `a.b` on some other handle can only name a member of
                    // this object through an alias; stay conservative.
                    if h.path.len() == 1 {
                        push_name(&seg.name.name, out);
                    }
                }
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                if let Some(n) = self.csp_member(csp, e) {
                    push_name(&n, out);
                    return true;
                }
                if member.name == "index" {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if let Some((it, _, _)) = &env.it {
                            if h.path.len() == 1 && &h.path[0].name.name == it {
                                return true;
                            }
                        }
                    }
                }
                self.csp_refs(csp, expr, env, out)
            }
            ExprKind::Call { func, args } => {
                let mut ok = match &func.kind {
                    ExprKind::MemberAccess { expr, .. } => self.csp_refs(csp, expr, env, out),
                    _ => false,
                };
                // A method call on a non-rand handle may still read members.
                if let ExprKind::MemberAccess { expr, .. } = &func.kind {
                    if self.csp_member(csp, expr).is_none()
                        && !matches!(&expr.kind, ExprKind::Ident(_))
                    {
                        ok = false;
                    }
                }
                for a in args {
                    ok &= self.csp_refs(csp, a, env, out);
                }
                ok
            }
            ExprKind::WithClause { expr, filter } => {
                let ok = self.csp_refs(csp, expr, env, out);
                // the filter reads `item`/`item.index`, covered by the array
                let mut tmp = Vec::new();
                let ok2 = self.csp_refs(csp, filter, env, &mut tmp);
                out.extend(tmp);
                ok && ok2
            }
            ExprKind::SystemCall { args, .. } => args
                .iter()
                .fold(true, |ok, a| self.csp_refs(csp, a, env, out) && ok),
            ExprKind::Unary { operand, .. } => self.csp_refs(csp, operand, env, out),
            ExprKind::Binary { left, right, .. } => {
                let a = self.csp_refs(csp, left, env, out);
                self.csp_refs(csp, right, env, out) && a
            }
            ExprKind::Paren(i) => self.csp_refs(csp, i, env, out),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                let b = self.csp_refs(csp, then_expr, env, out);
                self.csp_refs(csp, else_expr, env, out) && a && b
            }
            ExprKind::Inside { expr, ranges } => ranges
                .iter()
                .fold(self.csp_refs(csp, expr, env, out), |ok, r| {
                    self.csp_refs(csp, r, env, out) && ok
                }),
            ExprKind::Range(a, b) => {
                let x = self.csp_refs(csp, a, env, out);
                self.csp_refs(csp, b, env, out) && x
            }
            ExprKind::Index { expr, index } => {
                let x = self.csp_refs(csp, expr, env, out);
                self.csp_refs(csp, index, env, out) && x
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                let x = self.csp_refs(csp, expr, env, out);
                let y = self.csp_refs(csp, left, env, out);
                self.csp_refs(csp, right, env, out) && x && y
            }
            ExprKind::Concatenation(es) => es
                .iter()
                .fold(true, |ok, a| self.csp_refs(csp, a, env, out) && ok),
            ExprKind::Replication { count, exprs } => exprs
                .iter()
                .fold(self.csp_refs(csp, count, env, out), |ok, a| {
                    self.csp_refs(csp, a, env, out) && ok
                }),
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::TypeLiteral(_) => true,
            _ => false,
        }
    }

    fn csp_item_refs(
        &self,
        csp: &Csp,
        item: &ConstraintItem,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        match item {
            ConstraintItem::Expr(e) => self.csp_refs(csp, e, env, out),
            ConstraintItem::Inside { expr, range, .. } => {
                let mut ok = self.csp_refs(csp, expr, env, out);
                for r in range {
                    ok &= match r {
                        ConstraintRange::Value(e) => self.csp_refs(csp, e, env, out),
                        ConstraintRange::Range { lo, hi } => {
                            let a = self.csp_refs(csp, lo, env, out);
                            self.csp_refs(csp, hi, env, out) && a
                        }
                    };
                }
                ok
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                self.csp_item_refs(csp, constraint, env, out) && a
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                let b = self.csp_item_refs(csp, then_item, env, out);
                let c = else_item
                    .as_ref()
                    .is_none_or(|e| self.csp_item_refs(csp, e, env, out));
                a && b && c
            }
            ConstraintItem::Foreach { array, item, .. } => {
                // the loop index is not a variable; the array's elements are
                let a = self.csp_refs(csp, array, env, out);
                let mut inner = Vec::new();
                let b = self.csp_item_refs(csp, item, env, &mut inner);
                out.extend(inner);
                a && b
            }
            ConstraintItem::Solve { .. } => true,
            ConstraintItem::Soft(i) => self.csp_item_refs(csp, i, env, out),
            ConstraintItem::Block(items) => items
                .iter()
                .fold(true, |ok, i| self.csp_item_refs(csp, i, env, out) && ok),
            ConstraintItem::Unique { exprs, .. } => exprs
                .iter()
                .fold(true, |ok, e| self.csp_refs(csp, e, env, out) && ok),
        }
    }

    /// Reads no solver variable (and calls nothing opaque).
    fn csp_free(&self, csp: &Csp, e: &Expression, env: &Env) -> bool {
        let mut v = Vec::new();
        self.csp_refs(csp, e, env, &mut v) && v.is_empty()
    }

    /// Evaluate a variable-free expression with the foreach indices bound.
    fn csp_const_any(&mut self, e: &Expression, env: &Env) -> Value {
        let frame: HashMap<String, Value> = env.binds.iter().cloned().collect();
        self.push_local_frame(frame);
        let v = self.eval_expr(e);
        self.pop_local_frame();
        v
    }

    fn csp_const(&mut self, e: &Expression, env: &Env) -> Option<Value> {
        let v = self.csp_const_any(e, env);
        (!v.has_xz() && !v.is_real && v.width > 0 && v.width <= 64).then_some(v)
    }

    /// Arithmetic operand tree of `e`, or None when it is not linear.
    fn csp_ae(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let e = Self::unparen(e);
        // the `with` iterator and its index
        if let Some((it, v, i)) = &env.it {
            match &e.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && &h.path[0].name.name == it => {
                    return Some(Self::csp_var_ae(csp, *v));
                }
                ExprKind::MemberAccess { expr, member } if member.name == "index" => {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if h.path.len() == 1 && &h.path[0].name.name == it {
                            // §7.12.4: `index` is an int
                            let mut ix = Value::from_u64(*i as u64, 32);
                            ix.is_signed = true;
                            return Some(Ae::Const(ix));
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(n) = self.csp_member(csp, e) {
            if !env.binds.iter().any(|b| b.0 == n) {
                return csp.scalars.get(&n).map(|&v| Self::csp_var_ae(csp, v));
            }
        }
        match &e.kind {
            ExprKind::Unary {
                op: UnaryOp::Plus,
                operand,
            } => return self.csp_ae(csp, operand, env),
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => return Some(Ae::Neg(Box::new(self.csp_ae(csp, operand, env)?))),
            ExprKind::Binary {
                op: op @ (BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul),
                left,
                right,
            } => {
                let (l, r) = (self.csp_ae(csp, left, env)?, self.csp_ae(csp, right, env)?);
                let (l, r) = (Box::new(l), Box::new(r));
                return Some(match op {
                    BinaryOp::Add => Ae::Add(l, r),
                    BinaryOp::Sub => Ae::Sub(l, r),
                    _ => Ae::Mul(l, r),
                });
            }
            ExprKind::Index { expr, index } => {
                if let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n)) {
                    if !self.csp_free(csp, index, env) {
                        return None;
                    }
                    let i = self.csp_const(index, env)?.to_i64()?;
                    return a
                        .elems
                        .iter()
                        .find(|x| x.0 == i)
                        .map(|x| Self::csp_var_ae(csp, x.1));
                }
            }
            ExprKind::SystemCall { name, args } => match (name.as_str(), args.as_slice()) {
                ("$__xz_type_cast", [t, inner]) => {
                    let ExprKind::TypeLiteral(dt) = &t.kind else {
                        return None;
                    };
                    if is_type_real(dt) {
                        return None;
                    }
                    let w = resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    let s = is_type_signed(dt);
                    return self.csp_cast(csp, inner, w, Some(s), env);
                }
                ("$__xz_size_cast", [n, inner]) if self.csp_free(csp, n, env) => {
                    let w = self.csp_const(n, env)?.to_u64()? as u32;
                    return self.csp_cast(csp, inner, w, None, env);
                }
                // `T'(e)` with `T` a typedef/enum, or a constant size.
                ("$__xz_named_cast", [t, inner]) => {
                    let ExprKind::Ident(h) = &t.kind else {
                        return None;
                    };
                    let nm = h.path.last()?.name.name.as_str();
                    let (w, s) = if let Some(dt) = self.module.typedef_types.get(nm) {
                        if is_type_real(dt) {
                            return None;
                        }
                        (self.cast_context_width(dt), Some(is_type_signed(dt)))
                    } else if let Some(&w) = self.module.typedefs.get(nm) {
                        (w, Some(false))
                    } else if self.csp_free(csp, t, env) {
                        (self.csp_const(t, env)?.to_u64()? as u32, None)
                    } else {
                        return None;
                    };
                    return self.csp_cast(csp, inner, w, s, env);
                }
                ("$signed", [inner]) | ("$unsigned", [inner]) => {
                    let a = self.csp_ae(csp, inner, env)?;
                    let (iw, _) = Self::csp_ws(&a);
                    return self.csp_cast(csp, inner, iw, Some(name == "$signed"), env);
                }
                _ => {}
            },
            _ => {}
        }
        if let Some(sum) = self.csp_sum(csp, e, env) {
            return sum;
        }
        // `arr.size()` of a solver array is a constant here
        if let ExprKind::Call { func, args } = &e.kind {
            if let ExprKind::MemberAccess { expr, member } = &func.kind {
                if args.is_empty() && member.name == "size" {
                    if let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n)) {
                        let mut v = Value::from_u64(a.elems.len() as u64, 32);
                        v.is_signed = true;
                        return Some(Ae::Const(v));
                    }
                }
            }
        }
        if self.csp_free(csp, e, env) {
            return self.csp_const(e, env).map(Ae::Const);
        }
        None
    }

    fn csp_cast(
        &mut self,
        csp: &Csp,
        inner: &Expression,
        w: u32,
        s: Option<bool>,
        env: &Env,
    ) -> Option<Ae> {
        let a = self.csp_ae(csp, inner, env)?;
        // §6.24.1: the cast width is the operand's context, so the operand
        // is evaluated at no less than the target width.
        let (iw, is) = Self::csp_ws(&a);
        let iw = iw.max(w);
        if w == 0 || w > 64 || iw > 64 {
            return None;
        }
        Some(Ae::Cast {
            inner: Box::new(a),
            iw,
            is,
            w,
            s: s.unwrap_or(is),
        })
    }

    /// `arr.sum()` / `arr.sum() with (f)` / `arr.sum(it) with (f)` over a
    /// solver array. `Some(None)` for a reduction the model cannot express.
    fn csp_sum(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Option<Ae>> {
        let (call, filter) = match &e.kind {
            ExprKind::WithClause { expr, filter } => (expr.as_ref(), Some(filter.as_ref())),
            _ => (e, None),
        };
        let (recv, method, args): (&Expression, &str, &[Expression]) = match &call.kind {
            ExprKind::Call { func, args } => match &func.kind {
                ExprKind::MemberAccess { expr, member } => (expr, member.name.as_str(), args),
                _ => return None,
            },
            ExprKind::MemberAccess { expr, member } => (expr, member.name.as_str(), &[]),
            _ => return None,
        };
        let arr = csp.arrays.get(&self.csp_member(csp, recv)?)?.clone();
        if method != "sum" {
            return Some(None);
        }
        let it = match args {
            [] => "item".to_string(),
            [a] => match &Self::unparen(a).kind {
                ExprKind::Ident(h) if h.path.len() == 1 => h.path[0].name.name.clone(),
                _ => return Some(None),
            },
            _ => return Some(None),
        };
        let mut terms = Vec::with_capacity(arr.elems.len());
        for &(i, v) in &arr.elems {
            let t = match filter {
                None => Some(Self::csp_var_ae(csp, v)),
                Some(f) => {
                    let mut e2 = env.clone();
                    e2.it = Some((it.clone(), v, i));
                    self.csp_ae(csp, f, &e2)
                }
            };
            let Some(t) = t else {
                return Some(None);
            };
            terms.push(t);
        }
        let (w, s) = match (filter, terms.first()) {
            (Some(_), Some(t)) => Self::csp_ws(t),
            _ => (arr.width, arr.signed),
        };
        if terms.iter().any(|t| Self::csp_ws(t) != (w, s)) || w > 64 {
            return Some(None);
        }
        Some(Some(Ae::Sum { terms, w, s }))
    }

    fn csp_var_ae(csp: &Csp, v: usize) -> Ae {
        Ae::Var(v, csp.vars[v].width, csp.vars[v].signed)
    }

    /// Self-determined (width, signed) of an operand tree.
    fn csp_ws(a: &Ae) -> (u32, bool) {
        match a {
            Ae::Var(_, w, s) => (*w, *s),
            Ae::Const(v) => (v.width, v.is_signed),
            Ae::Neg(x) => Self::csp_ws(x),
            Ae::Add(l, r) | Ae::Sub(l, r) | Ae::Mul(l, r) => {
                let ((a, b), (c, d)) = (Self::csp_ws(l), Self::csp_ws(r));
                (a.max(c), b && d)
            }
            Ae::Sum { w, s, .. } | Ae::Cast { w, s, .. } => (*w, *s),
        }
    }

    /// Linear form of `a` in the context `(w, s)`, recording the fits that
    /// keep it exact.
    fn csp_lin(a: &Ae, w: u32, s: bool, fits: &mut Vec<Fit>) -> Option<Lin> {
        match a {
            Ae::Var(v, _, vs) => {
                // an unsigned context zero-extends a signed operand
                if *vs && !s {
                    fits.push(Fit {
                        lin: Lin::var(*v),
                        lo: 0,
                        hi: i128::MAX,
                    });
                }
                Some(Lin::var(*v))
            }
            Ae::Const(v) => {
                let raw = v.to_u64()?;
                Some(Lin::konst(Self::as_ctx(raw as i128, v.width, s)))
            }
            Ae::Neg(x) => Self::csp_lin(x, w, s, fits)?.scale(-1),
            Ae::Add(l, r) => Self::csp_lin(l, w, s, fits)?.plus(&Self::csp_lin(r, w, s, fits)?, 1),
            Ae::Sub(l, r) => Self::csp_lin(l, w, s, fits)?.plus(&Self::csp_lin(r, w, s, fits)?, -1),
            Ae::Mul(l, r) => {
                let (x, y) = (Self::csp_lin(l, w, s, fits)?, Self::csp_lin(r, w, s, fits)?);
                if x.is_const() {
                    y.scale(x.k)
                } else if y.is_const() {
                    x.scale(y.k)
                } else {
                    None
                }
            }
            Ae::Sum {
                terms,
                w: sw,
                s: ss,
            } => {
                let mut acc = Lin::konst(0);
                for t in terms {
                    acc = acc.plus(&Self::csp_lin(t, *sw, *ss, fits)?, 1)?;
                }
                Self::csp_leaf_fit(&acc, *sw, *ss, *sw, *ss, s, fits);
                Some(acc)
            }
            Ae::Cast {
                inner,
                iw,
                is,
                w: cw,
                s: cs,
            } => {
                let l = Self::csp_lin(inner, *iw, *is, fits)?;
                Self::csp_leaf_fit(&l, *iw, *is, *cw, *cs, s, fits);
                Some(l)
            }
        }
    }

    /// A sub-result computed at `(iw, is)`, converted to `(cw, cs)` and then
    /// extended into a context of signedness `ctx_s`, keeps its value when it
    /// lies in both ranges (and is non-negative when a signed value meets an
    /// unsigned context, which zero-extends).
    fn csp_leaf_fit(
        l: &Lin,
        iw: u32,
        is: bool,
        cw: u32,
        cs: bool,
        ctx_s: bool,
        fits: &mut Vec<Fit>,
    ) {
        let (a, b) = ws_range(iw, is);
        let (c, d) = ws_range(cw, cs);
        let lo = if cs && !ctx_s {
            a.max(c).max(0)
        } else {
            a.max(c)
        };
        fits.push(Fit {
            lin: l.clone(),
            lo,
            hi: b.min(d),
        });
    }
}
