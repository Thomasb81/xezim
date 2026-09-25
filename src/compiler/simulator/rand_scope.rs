//! §18.7 — name binding inside `obj.randomize() with { … }`.
//!
//! A name in the inline block binds first in the randomized OBJECT's class
//! scope and only then in the scope of the randomize() call; `local::name`
//! skips the object and binds in the call's scope directly (§18.7.1). The
//! solve itself runs with `this` switched to the object, where the caller's
//! locals and its class members no longer resolve. Every operand that binds
//! in the caller's scope is therefore a state variable of the solve: it is
//! evaluated once, before `this` switches, and replaced by a literal of its
//! value. Operands that bind in the object (its members, the receiver handle,
//! `this`, foreach iterators) are left for the solver.
use super::*;
use crate::ast::decl::DistWeight;

/// Where the root name of an operand binds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bind {
    /// The randomized object (or a foreach iterator): left to the solver.
    Object,
    /// The scope of the randomize() call.
    Caller,
    /// Neither: parameters, enum constants, package and module names —
    /// they resolve the same way in either scope.
    Neutral,
}

pub(super) struct CallerScope {
    /// Properties and value parameters of the object's class chain.
    obj_names: HashSet<String>,
    /// Plain name of the receiver (`req` in `req.randomize()`), which names
    /// the object itself.
    receiver: Option<String>,
    /// Class of the caller (`this`'s class, or the class of a static method).
    caller_class: Option<String>,
}

impl Simulator {
    /// Rewrite the inline constraints of `obj.randomize() with {…}` so every
    /// caller-scope operand is a literal (see the module docs). Called with
    /// the CALLER's `this` and local frame still active. Returns None when
    /// nothing binds in the caller's scope, so the original items are used.
    pub(super) fn freeze_caller_scope_refs(
        &mut self,
        handle: usize,
        receiver: Option<String>,
        items: &[ConstraintItem],
    ) -> Option<Vec<ConstraintItem>> {
        let class_name = self.heap.get(handle)?.as_ref()?.class_name.clone();
        let mut obj_names: HashSet<String> = HashSet::default();
        let mut cur = Some(class_name);
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            obj_names.extend(cd.properties.keys().cloned());
            obj_names.extend(cd.static_properties.iter().cloned());
            obj_names.extend(cd.param_defaults.iter().map(|(n, _)| n.clone()));
            cur = cd.extends.clone();
        }
        let caller_class = match self.this_stack.last().copied().flatten() {
            Some(h) => self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone()),
            None => self.class_context_stack.last().cloned().flatten(),
        };
        let cx = CallerScope {
            obj_names,
            receiver,
            caller_class,
        };
        let mut bound: Vec<String> = Vec::new();
        if !items
            .iter()
            .any(|it| self.item_has_caller_ref(it, &cx, &mut bound))
        {
            return None;
        }
        let mut out = items.to_vec();
        for it in &mut out {
            self.freeze_item(it, &cx, &mut bound);
        }
        Some(out)
    }

    /// Is `name` a member of the caller's class chain?
    fn caller_class_has(&self, cx: &CallerScope, name: &str) -> bool {
        let mut cur = cx.caller_class.clone();
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                return false;
            };
            if cd.properties.contains_key(name)
                || cd.static_properties.contains(name)
                || cd.queue_properties.contains_key(name)
                || cd.assoc_properties.contains_key(name)
            {
                return true;
            }
            cur = cd.extends.clone();
        }
        false
    }

    fn bind_of(&self, h: &HierarchicalIdentifier, cx: &CallerScope, bound: &[String]) -> Bind {
        let Some(first) = h.path.first() else {
            return Bind::Neutral;
        };
        if h.root.as_deref() == Some(LOCAL_SCOPE_ROOT) {
            return Bind::Caller;
        }
        if h.root.is_some() {
            return Bind::Neutral;
        }
        let n = first.name.name.as_str();
        if n == "this"
            || n == "super"
            || bound.iter().any(|b| b == n)
            || cx.receiver.as_deref() == Some(n)
            || cx.obj_names.contains(n)
        {
            return Bind::Object;
        }
        if self.local_stack.last().is_some_and(|f| f.contains_key(n))
            || self.caller_class_has(cx, n)
        {
            return Bind::Caller;
        }
        Bind::Neutral
    }

    /// Does `e` read any caller-scope name (read-only pre-scan)?
    fn expr_has_caller_ref(&self, e: &Expression, cx: &CallerScope, bound: &[String]) -> bool {
        let mut hit = false;
        Self::walk_operands(e, &mut |x| {
            if let ExprKind::Ident(h) = &x.kind {
                if self.bind_of(h, cx, bound) == Bind::Caller {
                    hit = true;
                }
            }
        });
        hit
    }

    /// Visit `e` and every sub-expression, including select expressions held
    /// on identifier path segments.
    fn walk_operands(e: &Expression, f: &mut dyn FnMut(&Expression)) {
        f(e);
        let mut sub = |x: &Expression| Self::walk_operands(x, f);
        match &e.kind {
            ExprKind::Ident(h) => {
                for seg in &h.path {
                    seg.selects.iter().for_each(&mut sub);
                }
            }
            ExprKind::Unary { operand, .. } => sub(operand),
            ExprKind::Binary { left, right, .. } => {
                sub(left);
                sub(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                sub(condition);
                sub(then_expr);
                sub(else_expr);
            }
            ExprKind::Concatenation(parts) => parts.iter().for_each(&mut sub),
            ExprKind::Replication { count, exprs } => {
                sub(count);
                exprs.iter().for_each(&mut sub);
            }
            ExprKind::Call { func, args } => {
                sub(func);
                args.iter().for_each(&mut sub);
            }
            ExprKind::SystemCall { args, .. } => args.iter().for_each(&mut sub),
            ExprKind::Inside { expr, ranges } => {
                sub(expr);
                ranges.iter().for_each(&mut sub);
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => sub(expr),
            ExprKind::Index { expr, index } => {
                sub(expr);
                sub(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                sub(expr);
                sub(left);
                sub(right);
            }
            ExprKind::Range(a, b) => {
                sub(a);
                sub(b);
            }
            _ => {}
        }
    }

    fn item_has_caller_ref(
        &self,
        it: &ConstraintItem,
        cx: &CallerScope,
        bound: &mut Vec<String>,
    ) -> bool {
        match it {
            ConstraintItem::Expr(e) => self.expr_has_caller_ref(e, cx, bound),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                self.expr_has_caller_ref(expr, cx, bound)
                    || range.iter().any(|r| match r {
                        ConstraintRange::Value(v) => self.expr_has_caller_ref(v, cx, bound),
                        ConstraintRange::Range { lo, hi } => {
                            self.expr_has_caller_ref(lo, cx, bound)
                                || self.expr_has_caller_ref(hi, cx, bound)
                        }
                    })
                    || dist_weights.iter().flatten().any(|w| match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => {
                            self.expr_has_caller_ref(e, cx, bound)
                        }
                    })
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                self.expr_has_caller_ref(condition, cx, bound)
                    || self.item_has_caller_ref(constraint, cx, bound)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                self.expr_has_caller_ref(condition, cx, bound)
                    || self.item_has_caller_ref(then_item, cx, bound)
                    || else_item
                        .as_ref()
                        .is_some_and(|e| self.item_has_caller_ref(e, cx, bound))
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                if self.expr_has_caller_ref(array, cx, bound) {
                    return true;
                }
                let depth = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                let r = self.item_has_caller_ref(item, cx, bound);
                bound.truncate(depth);
                r
            }
            ConstraintItem::Soft(inner) => self.item_has_caller_ref(inner, cx, bound),
            ConstraintItem::Block(items) => {
                items.iter().any(|i| self.item_has_caller_ref(i, cx, bound))
            }
            ConstraintItem::Unique { exprs, .. } => {
                exprs.iter().any(|e| self.expr_has_caller_ref(e, cx, bound))
            }
            ConstraintItem::Solve { .. } => false,
        }
    }

    fn freeze_item(&mut self, it: &mut ConstraintItem, cx: &CallerScope, bound: &mut Vec<String>) {
        match it {
            ConstraintItem::Expr(e) => self.freeze_expr(e, cx, bound),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                self.freeze_expr(expr, cx, bound);
                let mut out: Vec<ConstraintRange> = Vec::with_capacity(range.len());
                for mut r in std::mem::take(range) {
                    match &mut r {
                        ConstraintRange::Value(v) => {
                            if !*is_dist {
                                if let Some(elems) = self.caller_set_elems(v, cx, bound) {
                                    out.extend(elems.into_iter().map(ConstraintRange::Value));
                                    continue;
                                }
                            }
                            self.freeze_expr(v, cx, bound);
                        }
                        ConstraintRange::Range { lo, hi } => {
                            self.freeze_expr(lo, cx, bound);
                            self.freeze_expr(hi, cx, bound);
                        }
                    }
                    out.push(r);
                }
                *range = out;
                for w in dist_weights.iter_mut().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => {
                            self.freeze_expr(e, cx, bound)
                        }
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                self.freeze_expr(condition, cx, bound);
                self.freeze_item(constraint, cx, bound);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                self.freeze_expr(condition, cx, bound);
                self.freeze_item(then_item, cx, bound);
                if let Some(e) = else_item {
                    self.freeze_item(e, cx, bound);
                }
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                self.freeze_expr(array, cx, bound);
                let depth = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                self.freeze_item(item, cx, bound);
                bound.truncate(depth);
            }
            ConstraintItem::Soft(inner) => self.freeze_item(inner, cx, bound),
            ConstraintItem::Block(items) => {
                for i in items {
                    self.freeze_item(i, cx, bound);
                }
            }
            ConstraintItem::Unique { exprs, .. } => {
                for e in exprs {
                    self.freeze_expr(e, cx, bound);
                }
            }
            ConstraintItem::Solve { .. } => {}
        }
    }

    /// Where an OPERAND binds as a whole: `Some(true)` when it reads a
    /// caller-scope name and nothing of the object, `Some(false)` when it
    /// reads neither, None when it reads the object (or has a shape that
    /// cannot be evaluated ahead of the solve).
    fn operand_bind(&self, e: &Expression, cx: &CallerScope, bound: &[String]) -> Option<bool> {
        let all = |xs: &[&Expression]| -> Option<bool> {
            let mut any = false;
            for x in xs {
                any |= self.operand_bind(x, cx, bound)?;
            }
            Some(any)
        };
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::TypeLiteral(_) => {
                Some(false)
            }
            ExprKind::Ident(h) => {
                let own = match self.bind_of(h, cx, bound) {
                    Bind::Object => return None,
                    Bind::Caller => true,
                    Bind::Neutral => false,
                };
                let sels: Vec<&Expression> = h.path.iter().flat_map(|s| s.selects.iter()).collect();
                let sel = all(&sels)?;
                Some(own || sel)
            }
            ExprKind::Index { expr, index } => all(&[expr.as_ref(), index.as_ref()]),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => all(&[expr.as_ref(), left.as_ref(), right.as_ref()]),
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => {
                self.operand_bind(expr, cx, bound)
            }
            ExprKind::Unary { operand, .. } => self.operand_bind(operand, cx, bound),
            ExprKind::Binary { left, right, .. } => all(&[left.as_ref(), right.as_ref()]),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => all(&[condition.as_ref(), then_expr.as_ref(), else_expr.as_ref()]),
            ExprKind::Concatenation(parts) => all(&parts.iter().collect::<Vec<_>>()),
            ExprKind::SystemCall { args, .. } => all(&args.iter().collect::<Vec<_>>()),
            // A method of a caller-scope object (`q.size()`, `cfg.get()`);
            // a bare call binds in the object's class first, so it stays.
            ExprKind::Call { func, args } => {
                let recv = match &func.kind {
                    ExprKind::MemberAccess { expr, .. } => self.operand_bind(expr, cx, bound)?,
                    ExprKind::Ident(h) if h.path.len() > 1 => match self.bind_of(h, cx, bound) {
                        Bind::Object => return None,
                        Bind::Caller => true,
                        Bind::Neutral => false,
                    },
                    _ => return None,
                };
                Some(recv || all(&args.iter().collect::<Vec<_>>())?)
            }
            _ => None,
        }
    }

    /// Replace each maximal caller-scope operand of `e` with a literal.
    /// Operators are kept (not folded), so the context-determined width
    /// rules of §11.6 still apply to the frozen operands.
    fn freeze_expr(&mut self, e: &mut Expression, cx: &CallerScope, bound: &[String]) {
        let operand = matches!(
            e.kind,
            ExprKind::Ident(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Call { .. }
                | ExprKind::SystemCall { .. }
        );
        if operand && self.operand_bind(e, cx, bound) == Some(true) {
            // A whole unpacked collection has no literal form; it is left
            // as written (only its elements and methods are frozen).
            if matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
                && self.array_operand_name(e).is_some()
            {
                return;
            }
            let v = self.eval_expr(e);
            *e = Self::literal_of(&v, e.span);
            return;
        }
        match &mut e.kind {
            ExprKind::Ident(h) => {
                // `local::name` whose value has no literal form: bind the
                // plain name (the object never shadows it at this point).
                if h.root.as_deref() == Some(LOCAL_SCOPE_ROOT) {
                    h.root = None;
                }
                for seg in &mut h.path {
                    for s in &mut seg.selects {
                        self.freeze_expr(s, cx, bound);
                    }
                }
            }
            ExprKind::Unary { operand, .. } => self.freeze_expr(operand, cx, bound),
            ExprKind::Binary { left, right, .. } => {
                self.freeze_expr(left, cx, bound);
                self.freeze_expr(right, cx, bound);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.freeze_expr(condition, cx, bound);
                self.freeze_expr(then_expr, cx, bound);
                self.freeze_expr(else_expr, cx, bound);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.freeze_expr(p, cx, bound);
                }
            }
            ExprKind::Replication { count, exprs } => {
                self.freeze_expr(count, cx, bound);
                for x in exprs {
                    self.freeze_expr(x, cx, bound);
                }
            }
            ExprKind::Call { func, args } => {
                if let ExprKind::MemberAccess { expr, .. } = &mut func.kind {
                    self.freeze_expr(expr, cx, bound);
                }
                for a in args {
                    self.freeze_expr(a, cx, bound);
                }
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    self.freeze_expr(a, cx, bound);
                }
            }
            ExprKind::Inside { expr, ranges } => {
                self.freeze_expr(expr, cx, bound);
                let mut out: Vec<Expression> = Vec::with_capacity(ranges.len());
                for mut r in std::mem::take(ranges) {
                    if let Some(elems) = self.caller_set_elems(&r, cx, bound) {
                        out.extend(elems);
                        continue;
                    }
                    self.freeze_expr(&mut r, cx, bound);
                    out.push(r);
                }
                *ranges = out;
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => {
                self.freeze_expr(expr, cx, bound)
            }
            ExprKind::Index { expr, index } => {
                self.freeze_expr(expr, cx, bound);
                self.freeze_expr(index, cx, bound);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.freeze_expr(expr, cx, bound);
                self.freeze_expr(left, cx, bound);
                self.freeze_expr(right, cx, bound);
            }
            ExprKind::Range(a, b) => {
                self.freeze_expr(a, cx, bound);
                self.freeze_expr(b, cx, bound);
            }
            _ => {}
        }
    }

    /// §11.4.13: a caller-scope array/queue named in an `inside` set stands
    /// for its elements; return them as literals (none for an empty one).
    /// None when `r` is not such a collection.
    fn caller_set_elems(
        &mut self,
        r: &Expression,
        cx: &CallerScope,
        bound: &[String],
    ) -> Option<Vec<Expression>> {
        if !matches!(r.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
            || self.operand_bind(r, cx, bound) != Some(true)
        {
            return None;
        }
        let nm = self.array_operand_name(r)?;
        let n = self.get_queue_size(&nm);
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let v = self.get_signal_value_by_name(&format!("{}[{}]", nm, i))?;
            out.push(Self::literal_of(&v, r.span));
        }
        Some(out)
    }

    /// A literal expression that evaluates to exactly `v` (width, sign and
    /// every x/z bit included).
    pub(super) fn literal_of(v: &Value, span: crate::ast::Span) -> Expression {
        if v.is_real {
            return Expression::new(ExprKind::Number(NumberLiteral::Real(v.to_f64())), span);
        }
        let w = v.width.max(1);
        let mut digits = String::with_capacity(w as usize);
        for i in (0..w as usize).rev() {
            digits.push(match v.get_bit(i) {
                LogicBit::Zero => '0',
                LogicBit::One => '1',
                LogicBit::X => 'x',
                LogicBit::Z => 'z',
            });
        }
        let cached = if w <= 64 {
            v.resize(w).inline_bits().map(|(b, x)| (b, x, w))
        } else {
            None
        };
        Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: Some(w),
                signed: v.is_signed,
                base: NumberBase::Binary,
                value: digits,
                cached_val: Cell::new(cached),
            }),
            span,
        )
    }
}
