//! §9.4.2 event terms that watch one bit, or a constant part, of a vector.
//!
//! An always block whose event list names `v[3]` must wake only when that
//! bit changes. The edge machinery watches whole signals, so such a term is
//! either narrowed to a mask of physical bits at dispatch
//! (`Simulator::narrow_event_mask`, AnyEdge only) or, for `posedge`/`negedge`,
//! rewritten to a 1-bit alias net (`rewrite_edge_select_sensitivities`).
//! Anything else waits on the expression's value in a process.
//!
//! Inlining substitutes a port's connection for every use of the port, so
//! `@(dq[g])` on an `inout [7:0] dq` wired to `bus[d*8 +: 8]` reaches the
//! simulator as `bus[d*8 +: 8][g]` — a select OF a select, which neither
//! path recognized: each such block ran as an interpreted process parked on
//! the whole bus. `fold_port_select_events` resolves those shapes, before
//! classification, to the one bit (or contiguous run of bits) of the net
//! they denote, spelled `net[label]` / `net[l:r]` with the net's own
//! declared labels.
//!
//! The fold reproduces how the evaluator reads the original expression: a
//! select on a NAMED vector uses that vector's declared labels (§7.4.1); a
//! select on any other value (a part-select, a concatenation) counts
//! positions from its LSB. Ascending nets as select bases are left alone.

use super::*;

/// §7.4.1 declared `(left, right)` range of a plain 1-D packed vector of
/// `width` bits, or None when a select on `name` does not address single
/// bits by that range: an unpacked or associative collection, a packed
/// multi-dimensional vector (a select picks an element), a real, a string,
/// an event, or a layout on file that disagrees with the width.
pub(super) fn vector_decl_dim(
    module: &ElaboratedModule,
    name: &str,
    width: u32,
) -> Option<(i64, i64)> {
    if width == 0
        || module.arrays.contains_key(name)
        || module.arrays_2d.contains_key(name)
        || module.arrays_nd.contains_key(name)
        || module.associative_arrays.contains_key(name)
        || module.dynamic_arrays.contains(name)
        || module.queue_vars.contains(name)
        || module.string_signals.contains(name)
        || module.events.contains(name)
        || module.packed_signal_elem_widths.contains_key(name)
    {
        return None;
    }
    let span = |(l, r): (i64, i64)| (l - r).abs() + 1;
    if let Some(d) = module.packed_full_dims.get(name) {
        return match d.as_slice() {
            [dim] if span(*dim) == width as i64 => Some(*dim),
            _ => None,
        };
    }
    if let Some(&(lo, hi)) = module.ascending_packed.get(name) {
        return (span((lo, hi)) == width as i64).then_some((lo, hi));
    }
    Some((width as i64 - 1, 0))
}

/// §7.4.1 physical bit (0 = LSB) of declared label `label`.
pub(super) fn label_to_phys(dim: (i64, i64), label: i64) -> i64 {
    if dim.0 >= dim.1 {
        label - dim.1
    } else {
        dim.1 - label
    }
}

/// The declared label of physical bit `phys` (inverse of `label_to_phys`).
fn phys_to_label(dim: (i64, i64), phys: i64) -> i64 {
    if dim.0 >= dim.1 {
        dim.1 + phys
    } else {
        dim.1 - phys
    }
}

/// The lowest and highest declared label a constant select of a vector
/// with range `dim` covers, or None when the select runs against the
/// declared direction or is empty. §11.5.1: `b +: w` covers b..b+w-1 and
/// `b -: w` covers b-w+1..b, whatever the direction.
pub(super) fn select_label_span(
    dim: (i64, i64),
    kind: RangeKind,
    left: i64,
    right: i64,
) -> Option<(i64, i64)> {
    let desc = dim.0 >= dim.1;
    match kind {
        RangeKind::Constant => {
            if desc && left < right || !desc && left > right {
                return None;
            }
            Some((left.min(right), left.max(right)))
        }
        RangeKind::IndexedUp if right > 0 => Some((left, left.checked_add(right - 1)?)),
        RangeKind::IndexedDown if right > 0 => Some((left.checked_sub(right - 1)?, left)),
        _ => None,
    }
}

/// The label of the LSB a constant select of a vector with range `dim`
/// covers, and its width (see `select_label_span`).
fn part_select_labels(
    dim: (i64, i64),
    kind: RangeKind,
    left: i64,
    right: i64,
) -> Option<(i64, i64)> {
    let (lo, hi) = select_label_span(dim, kind, left, right)?;
    Some((if dim.0 >= dim.1 { lo } else { hi }, hi - lo + 1))
}

/// A literal or a parameter expression under `+ - *` (`v[W-1]`).
fn const_index(e: &Expression, params: &HashMap<String, Value>, scope: &str) -> Option<i64> {
    match &e.kind {
        ExprKind::Paren(inner) => const_index(inner, params, scope),
        ExprKind::Number(_) => Simulator::try_const_u64(e).and_then(|v| i64::try_from(v).ok()),
        ExprKind::Ident(h) if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) => {
            let raw = h
                .path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            let v = (!scope.is_empty())
                .then(|| params.get(&format!("{}.{}", scope, raw)))
                .flatten()
                .or_else(|| params.get(&raw))?;
            if v.has_xz() {
                return None;
            }
            i64::try_from(v.to_u64()?).ok()
        }
        ExprKind::Binary { op, left, right } => {
            let l = const_index(left, params, scope)?;
            let r = const_index(right, params, scope)?;
            match op {
                BinaryOp::Add => l.checked_add(r),
                BinaryOp::Sub => l.checked_sub(r),
                BinaryOp::Mul => l.checked_mul(r),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A plain vector a select can address bit-wise: its flat name and declared
/// range. Spelled as-is, or under the block's scope unless rooted (a
/// substituted port actual already names the absolute net); a name that
/// resolves BOTH ways is ambiguous and left alone.
struct VecRef {
    name: String,
    dim: (i64, i64),
    width: u32,
}

fn resolve_vector(module: &ElaboratedModule, e: &Expression, scope: &str) -> Option<VecRef> {
    let ExprKind::Ident(h) = &e.kind else {
        return None;
    };
    if h.path.last().is_some_and(|s| !s.selects.is_empty()) {
        return None;
    }
    let raw = Simulator::resolve_hier_name_static(h, module);
    let top = format!("{}.", module.name);
    let stripped = raw.strip_prefix(top.as_str()).map(str::to_string);
    let scoped = (h.root.is_none() && !scope.is_empty()).then(|| format!("{}.{}", scope, raw));
    let hit = |n: &str| module.signals.get(n).filter(|s| !s.is_real);
    let name = match (scoped.as_deref().and_then(hit), hit(&raw)) {
        (Some(_), Some(_)) if scoped.as_deref() != Some(raw.as_str()) => return None,
        (Some(_), _) => scoped?,
        (None, Some(_)) => raw,
        (None, None) => {
            let s = stripped?;
            hit(&s)?;
            s
        }
    };
    let width = module.signals.get(&name)?.width;
    let dim = vector_decl_dim(module, &name, width)?;
    Some(VecRef { name, dim, width })
}

/// The net bit at position `pos` (0 = LSB) of the VALUE of `e`, as
/// `(flat name, declared range, physical bit)`, following the evaluator's
/// reading of each shape. None for anything not made only of constant
/// selects and concatenations over plain descending vectors.
fn bit_at(
    module: &ElaboratedModule,
    e: &Expression,
    pos: i64,
    scope: &str,
) -> Option<(String, (i64, i64), i64)> {
    if pos < 0 {
        return None;
    }
    let params = &module.parameters;
    match &e.kind {
        ExprKind::Paren(inner) => bit_at(module, inner, pos, scope),
        ExprKind::Ident(_) => {
            let v = resolve_vector(module, e, scope)?;
            (pos < v.width as i64).then_some((v.name, v.dim, pos))
        }
        ExprKind::Index { expr: base, index } => {
            let k = const_index(index, params, scope)?;
            if pos != 0 {
                return None;
            }
            if matches!(base.kind, ExprKind::Ident(_)) {
                let v = resolve_vector(module, base, scope)?;
                // An ascending base is read through a mapping the port
                // shapes above it do not all share; keep it on the
                // expression path.
                if v.dim.0 < v.dim.1 {
                    return None;
                }
                let phys = label_to_phys(v.dim, k);
                (0..v.width as i64)
                    .contains(&phys)
                    .then_some((v.name, v.dim, phys))
            } else {
                bit_at(module, base, k, scope)
            }
        }
        ExprKind::RangeSelect {
            expr: base,
            kind,
            left,
            right,
        } => {
            let l = const_index(left, params, scope)?;
            let r = const_index(right, params, scope)?;
            if matches!(base.kind, ExprKind::Ident(_)) {
                let v = resolve_vector(module, base, scope)?;
                if v.dim.0 < v.dim.1 {
                    return None;
                }
                let (lsb, w) = part_select_labels(v.dim, *kind, l, r)?;
                if pos >= w {
                    return None;
                }
                let phys = label_to_phys(v.dim, lsb + pos);
                (0..v.width as i64)
                    .contains(&phys)
                    .then_some((v.name, v.dim, phys))
            } else {
                // A select on an unnamed value counts positions from its LSB.
                let (lsb, w) = part_select_labels((i64::MAX, 0), *kind, l, r)?;
                if pos >= w || lsb < 0 {
                    return None;
                }
                bit_at(module, base, lsb + pos, scope)
            }
        }
        ExprKind::Concatenation(parts) => {
            // §11.4.12: the LAST operand holds the LSBs.
            let mut rem = pos;
            for p in parts.iter().rev() {
                let w = value_width(module, p, scope)?;
                if rem < w {
                    return bit_at(module, p, rem, scope);
                }
                rem -= w;
            }
            None
        }
        _ => None,
    }
}

/// Width of a value `bit_at` can address.
fn value_width(module: &ElaboratedModule, e: &Expression, scope: &str) -> Option<i64> {
    let params = &module.parameters;
    match &e.kind {
        ExprKind::Paren(inner) => value_width(module, inner, scope),
        ExprKind::Ident(_) => Some(resolve_vector(module, e, scope)?.width as i64),
        ExprKind::Index { index, .. } => {
            const_index(index, params, scope)?;
            Some(1)
        }
        ExprKind::RangeSelect {
            kind, left, right, ..
        } => {
            let l = const_index(left, params, scope)?;
            let r = const_index(right, params, scope)?;
            Some(part_select_labels((i64::MAX, 0), *kind, l, r)?.1)
        }
        ExprKind::Concatenation(parts) => parts
            .iter()
            .map(|p| value_width(module, p, scope))
            .sum::<Option<i64>>(),
        _ => None,
    }
}

fn int_literal(v: i64, span: crate::ast::Span) -> Expression {
    let (neg, mag) = (v < 0, v.unsigned_abs());
    let lit = Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: None,
            signed: false,
            base: NumberBase::Decimal,
            value: mag.to_string(),
            cached_val: std::cell::Cell::new(None),
        }),
        span,
    );
    if neg {
        Expression::new(
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand: Box::new(lit),
            },
            span,
        )
    } else {
        lit
    }
}

fn rooted_ident(name: &str, span: crate::ast::Span) -> Expression {
    Expression::new(
        ExprKind::Ident(HierarchicalIdentifier {
            root: Some("$root".to_string()),
            path: vec![HierPathSegment {
                name: crate::ast::Identifier {
                    name: name.to_string(),
                    span,
                },
                selects: Vec::new(),
            }],
            span,
            cached_signal_id: std::cell::Cell::new(None),
            cached_resolved_name: std::cell::OnceCell::new(),
        }),
        span,
    )
}

/// The `net[label]` / `net[l:r]` spelling of event term `e` when it is a
/// select whose base is NOT itself a plain vector (the shapes port
/// substitution produces), or a concatenation, and its value is one bit or
/// a contiguous run of one net's bits. For an edge (`posedge`/`negedge`/
/// `edge`) only the LSB matters (§9.4.2), so any such value — a part-select
/// of a named vector included — folds to its LSB bit.
fn fold_event_expr(
    module: &ElaboratedModule,
    e: &Expression,
    edge: bool,
    scope: &str,
) -> Option<Expression> {
    let mut e = e;
    while let ExprKind::Paren(inner) = &e.kind {
        e = inner;
    }
    // `v[3]` on a named vector already has its own handling (the alias net
    // for an edge, the bit mask otherwise), and so does a level `v[7:4]`.
    let handled = match &e.kind {
        ExprKind::Index { expr, .. } => matches!(expr.kind, ExprKind::Ident(_)),
        ExprKind::RangeSelect { expr, .. } => !edge && matches!(expr.kind, ExprKind::Ident(_)),
        ExprKind::Concatenation(_) => false,
        _ => return None,
    };
    if handled {
        return None;
    }
    let w = if edge {
        1
    } else {
        value_width(module, e, scope)?
    };
    if w < 1 {
        return None;
    }
    let (name, dim, p0) = bit_at(module, e, 0, scope)?;
    for pos in 1..w {
        let (n, _, p) = bit_at(module, e, pos, scope)?;
        if n != name || p != p0 + pos {
            return None;
        }
    }
    let span = e.span;
    let net = rooted_ident(&name, span);
    Some(if w == 1 {
        Expression::new(
            ExprKind::Index {
                expr: Box::new(net),
                index: Box::new(int_literal(phys_to_label(dim, p0), span)),
            },
            span,
        )
    } else {
        Expression::new(
            ExprKind::RangeSelect {
                expr: Box::new(net),
                kind: RangeKind::Constant,
                left: Box::new(int_literal(phys_to_label(dim, p0 + w - 1), span)),
                right: Box::new(int_literal(phys_to_label(dim, p0), span)),
            },
            span,
        )
    })
}

/// Rewrite every always block's select-of-select event terms to the net
/// bits they denote (see the module docs). Runs on the materialized always
/// blocks, before the edge-select alias pass and classification.
pub(super) fn fold_port_select_events(module: &mut ElaboratedModule) {
    let dbg = std::env::var_os("XEZIM_DUMP_EDGE_SENS").is_some();
    let mut blocks = std::mem::take(&mut module.always_blocks);
    for ab in &mut blocks {
        let control = match &mut ab.stmt.kind {
            StatementKind::TimingControl { control, .. } => Some(control),
            StatementKind::SeqBlock { stmts, .. } => match stmts.first_mut().map(|s| &mut s.kind) {
                Some(StatementKind::TimingControl { control, .. }) => Some(control),
                _ => None,
            },
            _ => None,
        };
        let Some(TimingControl::Event(EventControl::EventExpr(exprs))) = control else {
            continue;
        };
        for ee in exprs.iter_mut() {
            if let Some(f) = fold_event_expr(module, &ee.expr, ee.edge.is_some(), &ab.scope) {
                if dbg {
                    let net = match &f.kind {
                        ExprKind::Index { expr, index } => {
                            format!(
                                "{:?}[{:?}]",
                                ident_name(expr),
                                Simulator::try_const_u64(index)
                            )
                        }
                        ExprKind::RangeSelect { expr, .. } => format!("{:?}[..]", ident_name(expr)),
                        _ => String::new(),
                    };
                    eprintln!("[EVFOLD] scope='{}' -> {}", ab.scope, net);
                }
                ee.expr = f;
            }
        }
    }
    module.always_blocks = blocks;
}

fn ident_name(e: &Expression) -> Option<&str> {
    match &e.kind {
        ExprKind::Ident(h) => h.path.first().map(|s| s.name.name.as_str()),
        _ => None,
    }
}
