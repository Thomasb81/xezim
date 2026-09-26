//! Code coverage: statement counts, collected when the run asks for them
//! (`--code-coverage`, `+cover`) and written to the results file beside the
//! functional coverage.
//!
//! Statements are instrumented at compile time, on the elaborated design,
//! before any block is lowered: a counted statement gets a `$__xz_cov(<n>)`
//! call in front of it. The bytecode compiler turns those into
//! `Insn::CovHit`; the interpreter counts them itself. Every execution path therefore counts the same way, and a run
//! without code coverage compiles exactly the design it did before.
//! Continuous assignments count in their comb entry (`ca_counter`).
use super::*;
use crate::ast::Span;

pub const KIND_STATEMENT: u8 = 1;
/// Result names of the kinds, bit `i` of a kind mask naming entry `i`.
const KIND_NAMES: [&str; 1] = ["statement"];
type Tallies = [Tally; KIND_NAMES.len()];

/// `$__xz_cov(<n>)`: counts counter `n` (a statement).
pub(crate) const HIT_TASK: &str = "$__xz_cov";
/// Library packages left out unless `--code-coverage-scope` names them.
const LIBRARY_PACKAGES: [&str; 2] = ["std", "uvm_pkg"];

/// What the command line asked for. `scopes` limits every kind to the
/// instances at or below the listed paths and to the listed packages (empty:
/// the whole design).
#[derive(Clone, Debug, Default)]
pub struct CodeCoverage {
    pub kinds: u8,
    pub scopes: Vec<String>,
}

static CONFIG: Mutex<Option<CodeCoverage>> = Mutex::new(None);

/// Select code coverage for the simulators built after this call; `None` or
/// no kinds turns it off.
pub fn set_code_coverage(cfg: Option<CodeCoverage>) {
    if let Ok(mut c) = CONFIG.lock() {
        *c = cfg.filter(|c| c.kinds != 0);
    }
}

pub(crate) fn code_coverage() -> Option<CodeCoverage> {
    CONFIG.lock().ok().and_then(|c| c.clone())
}

/// The kinds a `--code-coverage` list names: `stmt` (or `statement`, `s`)
/// or `all`, separated by commas.
pub fn parse_kinds(spec: &str) -> Result<u8, String> {
    let mut kinds = 0u8;
    for k in spec.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        kinds |= match k {
            "stmt" | "statement" | "s" => KIND_STATEMENT,
            "all" => KIND_STATEMENT,
            _ => {
                return Err(format!(
                    "unknown code coverage kind '{}' (want stmt or all)",
                    k
                ));
            }
        };
    }
    if kinds == 0 {
        return Err("--code-coverage needs at least one kind".to_string());
    }
    Ok(kinds)
}

/// The counter a coverage marker's first argument carries.
pub(crate) fn marker_id(e: &Expression) -> Option<u32> {
    match &e.kind {
        ExprKind::Number(NumberLiteral::Integer { value, .. }) => value.parse().ok(),
        _ => None,
    }
}

fn number(n: u32, span: Span) -> Expression {
    Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: None,
            signed: false,
            base: NumberBase::Decimal,
            value: n.to_string(),
            cached_val: Cell::new(None),
        }),
        span,
    )
}

fn hit(counter: u32, span: Span) -> Statement {
    Statement::new(
        StatementKind::Expr(Expression::new(
            ExprKind::SystemCall {
                name: HIT_TASK.to_string(),
                args: vec![number(counter, span)],
            },
            span,
        )),
        span,
    )
}

/// One statement list as a single statement.
fn block(mut v: Vec<Statement>, span: Span) -> Statement {
    if v.len() == 1 {
        v.pop().unwrap()
    } else {
        Statement::new(
            StatementKind::SeqBlock {
                name: None,
                stmts: v,
            },
            span,
        )
    }
}

/// A reported scope: a module instance, or the package (or `$unit`, or the
/// declaring module) that holds classes and subroutines outside instances.
struct Scope {
    /// Printed name: the hierarchical instance path, or the package name.
    name: String,
    unit: String,
    kind: &'static str,
    src: Option<u32>,
}

struct StmtSite {
    scope: u32,
    span: Span,
    kind: &'static str,
    counter: u32,
}

pub(crate) struct CodeCov {
    kinds: u8,
    limit: Vec<String>,
    scopes: Vec<Scope>,
    scope_index: HashMap<String, u32>,
    stmts: Vec<StmtSite>,
    counters: u32,
    /// Continuous assignment (origin span start, end, scope) -> its counter.
    ca_counter: HashMap<(usize, usize, String), u32>,
}

/// Rewrites one scope's statements; see the module docs.
struct Instr<'a> {
    cov: &'a mut CodeCov,
    kinds: u8,
    scope: u32,
}

impl Instr<'_> {
    fn counter(&mut self) -> u32 {
        let c = self.cov.counters;
        self.cov.counters += 1;
        c
    }

    fn count(&mut self, span: Span, kind: &'static str, out: &mut Vec<Statement>) {
        if self.kinds & KIND_STATEMENT == 0 {
            return;
        }
        let counter = self.counter();
        self.cov.stmts.push(StmtSite {
            scope: self.scope,
            span,
            kind,
            counter,
        });
        out.push(hit(counter, span));
    }

    /// A statement in a single-statement position.
    fn wrap(&mut self, s: Statement) -> Statement {
        let span = s.span;
        let mut v = Vec::new();
        self.stmt(s, &mut v);
        block(v, span)
    }

    fn stmts(&mut self, items: Vec<Statement>) -> Vec<Statement> {
        let mut v = Vec::with_capacity(items.len() * 2);
        for s in items {
            self.stmt(s, &mut v);
        }
        v
    }

    /// An `always` body: the construct itself counts each time the body
    /// starts (after its leading event or delay control, if any), and the
    /// body's statements count as usual.
    fn always(&mut self, s: Statement, kind: &'static str) -> Statement {
        let span = s.span;
        let mut v = Vec::new();
        match s.kind {
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } => {
                self.count(span, kind, &mut v);
                self.stmt(*inner, &mut v);
                Statement::new(
                    StatementKind::TimingControl {
                        control,
                        stmt: Box::new(block(v, span)),
                    },
                    span,
                )
            }
            other => {
                self.count(span, kind, &mut v);
                self.stmt(Statement::new(other, span), &mut v);
                block(v, span)
            }
        }
    }

    fn stmt(&mut self, s: Statement, out: &mut Vec<Statement>) {
        let span = s.span;
        let rebuilt = |kind| Statement::new(kind, span);
        match s.kind {
            StatementKind::SeqBlock { name, stmts } => {
                let stmts = self.stmts(stmts);
                out.push(rebuilt(StatementKind::SeqBlock { name, stmts }));
            }
            // Each fork branch stays one process.
            StatementKind::ParBlock {
                name,
                join_type,
                stmts,
            } => {
                let stmts = stmts.into_iter().map(|c| self.wrap(c)).collect();
                out.push(rebuilt(StatementKind::ParBlock {
                    name,
                    join_type,
                    stmts,
                }));
            }
            // Not statements themselves; their arms' statements count.
            StatementKind::If {
                unique_priority,
                condition,
                then_stmt,
                else_stmt,
            } => {
                let then_stmt = Box::new(self.wrap(*then_stmt));
                let else_stmt = else_stmt.map(|e| Box::new(self.wrap(*e)));
                out.push(rebuilt(StatementKind::If {
                    unique_priority,
                    condition,
                    then_stmt,
                    else_stmt,
                }));
            }
            StatementKind::Case {
                unique_priority,
                kind,
                expr,
                mut items,
            } => {
                for it in items.iter_mut() {
                    let st =
                        std::mem::replace(&mut it.stmt, Statement::new(StatementKind::Null, span));
                    it.stmt = self.wrap(st);
                }
                out.push(rebuilt(StatementKind::Case {
                    unique_priority,
                    kind,
                    expr,
                    items,
                }));
            }
            StatementKind::RandCase { items } => {
                let items = items
                    .into_iter()
                    .map(|(w, st)| (w, self.wrap(st)))
                    .collect();
                out.push(rebuilt(StatementKind::RandCase { items }));
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                self.count(span, "for", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::For {
                    init,
                    condition,
                    step,
                    body,
                }));
            }
            StatementKind::Foreach { array, vars, body } => {
                self.count(span, "foreach", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::Foreach { array, vars, body }));
            }
            StatementKind::While { condition, body } => {
                self.count(span, "while", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::While { condition, body }));
            }
            StatementKind::DoWhile { body, condition } => {
                self.count(span, "do", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::DoWhile { body, condition }));
            }
            StatementKind::Repeat { count, body } => {
                self.count(span, "repeat", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::Repeat { count, body }));
            }
            StatementKind::Forever { body } => {
                self.count(span, "forever", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::Forever { body }));
            }
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } => {
                let kind = match &control {
                    TimingControl::Delay(_) => "delay",
                    TimingControl::Event(_) => "event",
                };
                self.count(span, kind, out);
                let inner = Box::new(self.wrap(*inner));
                out.push(rebuilt(StatementKind::TimingControl {
                    control,
                    stmt: inner,
                }));
            }
            StatementKind::Wait {
                condition,
                stmt: inner,
            } => {
                self.count(span, "wait", out);
                let inner = Box::new(self.wrap(*inner));
                out.push(rebuilt(StatementKind::Wait {
                    condition,
                    stmt: inner,
                }));
            }
            // Not statements of their own: declarations, and the markers and
            // bodies the executor and other constructs own.
            kind @ (StatementKind::Null
            | StatementKind::VarDecl { .. }
            | StatementKind::Typedef(_)
            | StatementKind::Coverpoint { .. }
            | StatementKind::Cross { .. }
            | StatementKind::ScopePop
            | StatementKind::LoopStep
            | StatementKind::ForeachTail { .. }
            | StatementKind::ForeverTail { .. }
            | StatementKind::RsAction { .. }
            | StatementKind::RsReturn) => out.push(rebuilt(kind)),
            kind => {
                self.count(span, "statement", out);
                out.push(rebuilt(kind));
            }
        }
    }
}

/// Newline offsets of each retained source text, built on first use.
struct Lines<'a> {
    module: &'a ElaboratedModule,
    nl: HashMap<usize, Vec<usize>>,
}

impl Lines<'_> {
    /// File and line of `span`, in the text `src` names or else the only
    /// text long enough to contain it.
    fn locate(&mut self, span: Span, src: Option<u32>) -> Option<(String, u32)> {
        let texts = &self.module.source_texts;
        let i = match src
            .map(|s| s as usize)
            .filter(|&s| texts.get(s).is_some_and(|t| span.start < t.len()))
        {
            Some(i) => i,
            None => {
                let mut fits = texts
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| span.start < t.len());
                let (i, _) = fits.next()?;
                if fits.next().is_some() {
                    return None;
                }
                i
            }
        };
        let nl = self.nl.entry(i).or_insert_with(|| {
            texts[i]
                .bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(p, _)| p)
                .collect()
        });
        let out_line = nl.partition_point(|&p| p < span.start);
        if let Some(m) = self.module.source_line_maps.get(i).and_then(|m| m.as_ref()) {
            if let Some((f, l)) = m.file_line(out_line) {
                return Some((f.to_string(), l));
            }
        }
        let file = self.module.source_files.get(i).filter(|f| !f.is_empty())?;
        Some((file.clone(), out_line as u32 + 1))
    }
}

/// One reported statement after locating and merging.
#[derive(Clone)]
struct StmtRow {
    file: String,
    line: u32,
    start: usize,
    kind: &'static str,
    count: u64,
}

#[derive(Default)]
struct Report {
    stmts: Vec<StmtRow>,
}

#[derive(Clone, Copy, Default)]
struct Tally {
    covered: u64,
    total: u64,
}

impl Tally {
    fn add(&mut self, o: Tally) {
        self.covered += o.covered;
        self.total += o.total;
    }

    fn json(&self) -> String {
        if self.total == 0 {
            "{\"covered\": 0, \"total\": 0, \"percent\": null}".to_string()
        } else {
            format!(
                "{{\"covered\": {}, \"total\": {}, \"percent\": {:.2}}}",
                self.covered,
                self.total,
                self.percent()
            )
        }
    }

    fn percent(&self) -> f64 {
        100.0 * self.covered as f64 / self.total.max(1) as f64
    }

    fn text(&self) -> String {
        if self.total == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{} ({:.2}%)", self.covered, self.total, self.percent())
        }
    }
}

impl Report {
    fn tallies(&self) -> Tallies {
        let mut t = Tallies::default();
        for s in &self.stmts {
            t[0].total += 1;
            t[0].covered += (s.count > 0) as u64;
        }
        t
    }

    /// Sort, and add up rows that name the same source construct (the
    /// copies a parameterized class or an expanded assignment makes).
    fn normalize(&mut self) {
        self.stmts.sort_by(|a, b| {
            (&a.file, a.line, a.start, a.kind).cmp(&(&b.file, b.line, b.start, b.kind))
        });
        let mut merged: Vec<StmtRow> = Vec::with_capacity(self.stmts.len());
        for s in self.stmts.drain(..) {
            match merged.last_mut() {
                Some(m) if m.file == s.file && m.start == s.start && m.kind == s.kind => {
                    m.count += s.count
                }
                _ => merged.push(s),
            }
        }
        self.stmts = merged;
    }

    /// Add another instance of the same design unit (rows keyed by source
    /// position).
    fn merge(&mut self, o: &Report) {
        self.stmts.extend(o.stmts.iter().cloned());
        self.normalize();
    }

    fn json(&self, kinds: u8, indent: &str, out: &mut String) {
        let t = self.tallies();
        for (i, name) in KIND_NAMES.iter().enumerate() {
            if kinds & (1 << i) != 0 {
                out.push_str(&format!("{}\"{}\": {},\n", indent, name, t[i].json()));
            }
        }
        if kinds & KIND_STATEMENT != 0 {
            out.push_str(&format!("{}\"statements\": [", indent));
            for (i, s) in self.stmts.iter().enumerate() {
                out.push_str(&format!(
                    "{}{{\"file\": \"{}\", \"line\": {}, \"kind\": \"{}\", \"count\": {}}}",
                    if i == 0 { "" } else { ", " },
                    json_escape(&s.file),
                    s.line,
                    s.kind,
                    s.count
                ));
            }
            out.push_str("],\n");
        }
        // Drop the trailing comma of the last member.
        if out.ends_with(",\n") {
            out.truncate(out.len() - 2);
            out.push('\n');
        }
    }
}

impl Simulator {
    /// Instrument the design for the code coverage the command line asked
    /// for. Runs first in `compile`, before any block is lowered.
    pub(super) fn instrument_code_coverage(&mut self) {
        let Some(cfg) = code_coverage() else {
            return;
        };
        let kinds = cfg.kinds;
        // Every block must exist as an AST to be rewritten; the lazy forms
        // are only a memory saving.
        self.module.materialize_pending();
        let mut cov = CodeCov {
            kinds,
            limit: cfg.scopes,
            scopes: Vec::new(),
            scope_index: HashMap::default(),
            stmts: Vec::new(),
            counters: 0,
            ca_counter: HashMap::default(),
        };
        let inst_paths: HashSet<String> = self
            .module
            .instances
            .iter()
            .map(|i| i.path.clone())
            .collect();
        if kinds & KIND_STATEMENT != 0 {
            let mut initial = std::mem::take(&mut self.module.initial_blocks);
            for ib in initial.iter_mut() {
                if Self::cov_synthesized_initial(&ib.stmt) {
                    continue;
                }
                if let Some(sc) = self.cov_instance_scope(&mut cov, &inst_paths, &ib.scope) {
                    let span = ib.stmt.span;
                    let s =
                        std::mem::replace(&mut ib.stmt, Statement::new(StatementKind::Null, span));
                    ib.stmt = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .wrap(s);
                }
            }
            self.module.initial_blocks = initial;
            let mut program = std::mem::take(&mut self.module.program_initial_blocks);
            for ib in program.iter_mut() {
                if let Some(sc) = self.cov_instance_scope(&mut cov, &inst_paths, &ib.scope) {
                    let span = ib.stmt.span;
                    let s =
                        std::mem::replace(&mut ib.stmt, Statement::new(StatementKind::Null, span));
                    ib.stmt = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .wrap(s);
                }
            }
            self.module.program_initial_blocks = program;
            let mut fin = std::mem::take(&mut self.module.final_blocks);
            for fb in fin.iter_mut() {
                if let Some(sc) = self.cov_instance_scope(&mut cov, &inst_paths, &fb.scope) {
                    let span = fb.stmt.span;
                    let s =
                        std::mem::replace(&mut fb.stmt, Statement::new(StatementKind::Null, span));
                    fb.stmt = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .wrap(s);
                }
            }
            self.module.final_blocks = fin;
            let mut always = std::mem::take(&mut self.module.always_blocks);
            for ab in always.iter_mut() {
                if let Some(sc) = self.cov_instance_scope(&mut cov, &inst_paths, &ab.scope) {
                    let kind = match ab.kind {
                        crate::ast::decl::AlwaysKind::Always => "always",
                        crate::ast::decl::AlwaysKind::AlwaysComb => "always_comb",
                        crate::ast::decl::AlwaysKind::AlwaysFf => "always_ff",
                        crate::ast::decl::AlwaysKind::AlwaysLatch => "always_latch",
                    };
                    let span = ab.stmt.span;
                    let s =
                        std::mem::replace(&mut ab.stmt, Statement::new(StatementKind::Null, span));
                    ab.stmt = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .always(s, kind);
                }
            }
            self.module.always_blocks = always;
            // Subroutines: a package's belong to the package, the others to
            // the instance their qualified name starts with.
            // A bare-named copy of an instance's subroutine (same source
            // span) is not the top module's own.
            let dotted: HashSet<(String, usize)> = self
                .module
                .functions
                .iter()
                .map(|(k, f)| (k, f.span))
                .chain(self.module.tasks.iter().map(|(k, t)| (k, t.span)))
                .filter_map(|(k, sp)| {
                    let (_, leaf) = k.rsplit_once('.')?;
                    Some((leaf.to_string(), sp.start))
                })
                .collect();
            let copy = |k: &str, sp: Span| {
                !k.contains('.') && !k.contains("::") && dotted.contains(&(k.to_string(), sp.start))
            };
            let mut functions = std::mem::take(&mut self.module.functions);
            for (key, fd) in functions.iter_mut() {
                if copy(key, fd.span) && !self.module.pkg_subr_owner.contains_key(key.as_str()) {
                    continue;
                }
                if let Some(sc) = self.cov_subroutine_scope(&mut cov, &inst_paths, key) {
                    let items = std::mem::take(&mut fd.items);
                    fd.items = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .stmts(items);
                }
            }
            self.module.functions = functions;
            let mut tasks = std::mem::take(&mut self.module.tasks);
            for (key, td) in tasks.iter_mut() {
                if copy(key, td.span) && !self.module.pkg_subr_owner.contains_key(key.as_str()) {
                    continue;
                }
                if let Some(sc) = self.cov_subroutine_scope(&mut cov, &inst_paths, key) {
                    let items = std::mem::take(&mut td.items);
                    td.items = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .stmts(items);
                }
            }
            self.module.tasks = tasks;
            self.cov_instrument_classes(&mut cov);
            let mut cas = std::mem::take(&mut self.module.continuous_assigns);
            for ca in cas.iter_mut() {
                let Some((span, scope)) = ca.origin.clone() else {
                    continue;
                };
                let scope = if scope.is_empty() {
                    Self::cov_lhs_base(&ca.lhs)
                        .and_then(|n| n.rsplit_once('.').map(|(p, _)| p.to_string()))
                        .unwrap_or_default()
                } else {
                    scope
                };
                let Some(sc) = self.cov_instance_scope(&mut cov, &inst_paths, &scope) else {
                    continue;
                };
                let mut ins = Instr {
                    cov: &mut cov,
                    kinds,
                    scope: sc,
                };
                if kinds & KIND_STATEMENT != 0 {
                    let key = (span.start, span.end, scope);
                    if !ins.cov.ca_counter.contains_key(&key) {
                        let counter = ins.counter();
                        ins.cov.stmts.push(StmtSite {
                            scope: sc,
                            span,
                            kind: "assign",
                            counter,
                        });
                        ins.cov.ca_counter.insert(key, counter);
                    }
                }
            }
            self.module.continuous_assigns = cas;
        }
        self.code_cov_hits = vec![0; cov.counters as usize];
        self.code_cov = Some(Box::new(cov));
    }

    /// An initial block elaboration made rather than the source: a
    /// variable declaration's initializer, or a module-level assertion.
    fn cov_synthesized_initial(s: &Statement) -> bool {
        let dummy = |sp: Span| sp.start == 0 && sp.end == 0;
        match &s.kind {
            _ if dummy(s.span) => true,
            StatementKind::BlockingAssign { lvalue, .. } => dummy(lvalue.span),
            StatementKind::Assertion(a) => a.is_property || a.deferred.is_some(),
            _ => false,
        }
    }

    /// The signal-name base of a continuous assignment's target.
    fn cov_lhs_base(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => Some(
                h.path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            ExprKind::Index { expr, .. }
            | ExprKind::RangeSelect { expr, .. }
            | ExprKind::MemberAccess { expr, .. }
            | ExprKind::Paren(expr) => Self::cov_lhs_base(expr),
            _ => None,
        }
    }

    /// The instance a block scope (`u0.u1.genblk2`) belongs to: its longest
    /// prefix that names an instance, else the top. `None` when the command
    /// line's scope limit leaves it out.
    fn cov_instance_scope(
        &self,
        cov: &mut CodeCov,
        inst_paths: &HashSet<String>,
        scope: &str,
    ) -> Option<u32> {
        let mut path = scope;
        let inst = loop {
            if path.is_empty() || inst_paths.contains(path) {
                break path;
            }
            match path.rsplit_once('.') {
                Some((p, _)) => path = p,
                None => path = "",
            }
        };
        let wrapper = self.module.name == xezim_core::MULTI_TOP_WRAPPER;
        let name = if wrapper {
            inst.to_string()
        } else if inst.is_empty() {
            self.module.name.clone()
        } else {
            format!("{}.{}", self.module.name, inst)
        };
        if name.is_empty() {
            return None;
        }
        if !cov.limit.is_empty()
            && !cov.limit.iter().any(|l| {
                name == *l
                    || name.starts_with(&format!("{}.", l))
                    || (!wrapper && inst == l.as_str())
                    || (!wrapper && inst.starts_with(&format!("{}.", l)))
            })
        {
            return None;
        }
        let key = format!("i:{}", inst);
        if let Some(&i) = cov.scope_index.get(&key) {
            return Some(i);
        }
        let unit = if inst.is_empty() {
            self.module.name.clone()
        } else {
            self.module
                .instances
                .iter()
                .find(|i| i.path == inst)
                .map(|i| i.def_name.clone())
                .unwrap_or_default()
        };
        let src = self.module.src_file_of_module.get(&unit).copied();
        let i = cov.scopes.len() as u32;
        cov.scopes.push(Scope {
            name,
            unit,
            kind: "instance",
            src,
        });
        cov.scope_index.insert(key, i);
        Some(i)
    }

    /// A scope outside the instance tree: a package, `$unit`, or the module
    /// that declares a class. Library packages count only when named.
    fn cov_package_scope(
        &self,
        cov: &mut CodeCov,
        name: &str,
        src_key: &str,
        kind: &'static str,
    ) -> Option<u32> {
        let listed = cov.limit.iter().any(|l| l == name);
        if !cov.limit.is_empty() && !listed {
            return None;
        }
        if !listed && LIBRARY_PACKAGES.contains(&name) {
            return None;
        }
        let key = format!("p:{}", name);
        if let Some(&i) = cov.scope_index.get(&key) {
            return Some(i);
        }
        let src = self
            .module
            .src_file_of_module
            .get(src_key)
            .or_else(|| self.module.src_file_of_module.get(name))
            .copied();
        let i = cov.scopes.len() as u32;
        cov.scopes.push(Scope {
            name: name.to_string(),
            unit: name.to_string(),
            kind,
            src,
        });
        cov.scope_index.insert(key, i);
        Some(i)
    }

    fn cov_subroutine_scope(
        &self,
        cov: &mut CodeCov,
        inst_paths: &HashSet<String>,
        key: &str,
    ) -> Option<u32> {
        if let Some((pkg, _)) = key.split_once("::") {
            return self.cov_package_scope(cov, pkg, pkg, "package");
        }
        if let Some(pkg) = self.module.pkg_subr_owner.get(key) {
            let pkg = pkg.clone();
            return self.cov_package_scope(cov, &pkg, &pkg, "package");
        }
        let scope = key.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        self.cov_instance_scope(cov, inst_paths, scope)
    }

    fn cov_instrument_classes(&mut self, cov: &mut CodeCov) {
        let kinds = cov.kinds;
        let mut classes = std::mem::take(&mut self.module.classes);
        let mut done: HashMap<usize, Arc<crate::compiler::elaborate::ElaboratedClass>> =
            HashMap::default();
        let mut keys: Vec<String> = classes.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let Some(arc) = classes.get(&key) else {
                continue;
            };
            let ptr = Arc::as_ptr(arc) as usize;
            if let Some(same) = done.get(&ptr) {
                classes.insert(key, same.clone());
                continue;
            }
            let (scope_name, src_key, kind) = match self.module.class_decl_pkg.get(&arc.name) {
                Some(p) => (p.clone(), p.clone(), "package"),
                None => match &arc.declaring_module {
                    Some(m) => (m.clone(), m.clone(), "module"),
                    None => ("$unit".to_string(), arc.name.clone(), "unit"),
                },
            };
            let Some(sc) = self.cov_package_scope(cov, &scope_name, &src_key, kind) else {
                continue;
            };
            if scope_name == "$unit" || cov.scopes[sc as usize].src.is_none() {
                // A file-scope class: its own entry locates its methods.
                if let Some(&f) = self.module.src_file_of_module.get(&arc.name) {
                    if cov.scopes[sc as usize].src.is_none() {
                        cov.scopes[sc as usize].src = Some(f);
                    }
                }
            }
            let mut arc = classes.remove(&key).unwrap();
            let cls = Arc::make_mut(&mut arc);
            let mut names: Vec<String> = cls.methods.keys().cloned().collect();
            names.sort();
            for m in names {
                let Some(method) = cls.methods.get_mut(&m) else {
                    continue;
                };
                let items = match &mut method.kind {
                    crate::ast::decl::ClassMethodKind::Function(fd)
                    | crate::ast::decl::ClassMethodKind::Extern(fd) => &mut fd.items,
                    crate::ast::decl::ClassMethodKind::Task(td) => &mut td.items,
                    crate::ast::decl::ClassMethodKind::PureVirtual(_) => continue,
                };
                let body = std::mem::take(items);
                *items = Instr {
                    cov: &mut *cov,
                    kinds,
                    scope: sc,
                }
                .stmts(body);
            }
            done.insert(ptr, arc.clone());
            classes.insert(key, arc);
        }
        self.module.classes = classes;
    }

    /// The comb-entry counter of an instrumented continuous assignment.
    pub(super) fn cov_ca_counter(
        &self,
        ca: &crate::compiler::elaborate::ContinuousAssignment,
    ) -> Option<u32> {
        let cov = self.code_cov.as_ref()?;
        if cov.kinds & KIND_STATEMENT == 0 {
            return None;
        }
        let (span, scope) = ca.origin.as_ref()?;
        if let Some(&c) = cov.ca_counter.get(&(span.start, span.end, scope.clone())) {
            return Some(c);
        }
        let inferred = Self::cov_lhs_base(&ca.lhs)
            .and_then(|n| n.rsplit_once('.').map(|(p, _)| p.to_string()))
            .unwrap_or_default();
        cov.ca_counter
            .get(&(span.start, span.end, inferred))
            .copied()
    }

    /// Count counter `c` (`Insn::CovHit` / `TsInsn::CovHit`).
    #[inline]
    pub(super) fn cov_count(&mut self, c: u32) {
        if let Some(h) = self.code_cov_hits.get_mut(c as usize) {
            *h += 1;
        }
    }

    /// `TsInsn::CovHit`: count, and note the count on the save list so a
    /// bail takes it back (`ts_restore_saved`) before the block runs again.
    #[inline]
    pub(super) fn ts_cov_hit(&mut self, c: u32) {
        self.cov_count(c);
        self.ts_save_list.push((c | TS_SAVE_COV, 0, 0));
    }

    /// Count a coverage marker's counter (interpreter side).
    pub(super) fn cov_hit(&mut self, arg: Option<&Expression>) {
        if let Some(c) = arg
            .and_then(marker_id)
            .and_then(|id| self.code_cov_hits.get_mut(id as usize))
        {
            *c += 1;
        }
    }

    fn cov_report(&self) -> Vec<Report> {
        let Some(cov) = self.code_cov.as_ref() else {
            return Vec::new();
        };
        let mut lines = Lines {
            module: &self.module,
            nl: HashMap::default(),
        };
        let mut reports: Vec<Report> = (0..cov.scopes.len()).map(|_| Report::default()).collect();
        let count = |c: u32| self.code_cov_hits.get(c as usize).copied().unwrap_or(0);
        for s in &cov.stmts {
            let src = cov.scopes[s.scope as usize].src;
            let (file, line) = lines.locate(s.span, src).unwrap_or_default();
            reports[s.scope as usize].stmts.push(StmtRow {
                file,
                line,
                start: s.span.start,
                kind: s.kind,
                count: count(s.counter),
            });
        }
        for r in reports.iter_mut() {
            r.normalize();
        }
        reports
    }

    /// The `code_coverage` member of the results file, and the `[COV]`
    /// summary lines.
    pub(super) fn code_coverage_json(&self) -> Option<String> {
        let cov = self.code_cov.as_ref()?;
        let kinds = cov.kinds;
        let reports = self.cov_report();
        let mut order: Vec<usize> = (0..cov.scopes.len()).collect();
        order.sort_by(|&a, &b| {
            (cov.scopes[a].kind, &cov.scopes[a].name)
                .cmp(&(cov.scopes[b].kind, &cov.scopes[b].name))
        });
        let mut total = Tallies::default();
        for r in &reports {
            for (t, x) in total.iter_mut().zip(r.tallies()) {
                t.add(x);
            }
        }
        let kind_names: Vec<&str> = KIND_NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| kinds & (1 << i) != 0)
            .map(|(_, n)| *n)
            .collect();
        let summary = |t: &Tallies| -> String {
            KIND_NAMES
                .iter()
                .enumerate()
                .filter(|(i, _)| kinds & (1 << i) != 0)
                .map(|(i, n)| format!("{} {}", n, t[i].text()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        chatter!("[COV] code coverage: {}", summary(&total));
        const SHOWN: usize = 50;
        for &i in order.iter().take(SHOWN) {
            let s = &cov.scopes[i];
            chatter!(
                "[COV]   {} ({}): {}",
                s.name,
                s.unit,
                summary(&reports[i].tallies())
            );
        }
        if order.len() > SHOWN {
            chatter!(
                "[COV]   ... {} more scopes in the results file",
                order.len() - SHOWN
            );
        }
        let mut json = String::new();
        json.push_str("  \"code_coverage\": {\n");
        json.push_str(&format!(
            "    \"kinds\": [{}],\n",
            kind_names
                .iter()
                .map(|k| format!("\"{}\"", k))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (i, n) in KIND_NAMES.iter().enumerate() {
            if kinds & (1 << i) != 0 {
                json.push_str(&format!("    \"{}\": {},\n", n, total[i].json()));
            }
        }
        json.push_str("    \"scopes\": [\n");
        for (j, &i) in order.iter().enumerate() {
            let s = &cov.scopes[i];
            json.push_str(&format!(
                "      {{\n        \"scope\": \"{}\",\n        \"design_unit\": \"{}\",\n        \"kind\": \"{}\",\n",
                json_escape(&s.name),
                json_escape(&s.unit),
                s.kind
            ));
            reports[i].json(kinds, "        ", &mut json);
            json.push_str(if j + 1 < order.len() {
                "      },\n"
            } else {
                "      }\n"
            });
        }
        json.push_str("    ],\n");
        // Design units: every instance of a module added together.
        let mut units: BTreeMap<String, (usize, Report)> = BTreeMap::new();
        for &i in &order {
            let s = &cov.scopes[i];
            let e = units
                .entry(s.unit.clone())
                .or_insert((0, Report::default()));
            e.0 += 1;
            e.1.merge(&reports[i]);
        }
        json.push_str("    \"design_units\": [\n");
        let n_units = units.len();
        for (j, (unit, (n, r))) in units.iter().enumerate() {
            json.push_str(&format!(
                "      {{\n        \"design_unit\": \"{}\",\n        \"instances\": {},\n",
                json_escape(unit),
                n
            ));
            r.json(kinds, "        ", &mut json);
            json.push_str(if j + 1 < n_units {
                "      },\n"
            } else {
                "      }\n"
            });
        }
        json.push_str("    ]\n  }");
        Some(json)
    }
}
