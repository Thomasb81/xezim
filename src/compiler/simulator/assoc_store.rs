//! Native `BTreeMap`-backed storage for SystemVerilog associative arrays
//! (§7.8–§7.9) and identity tracking for queue element `ref` aliases (§7.10.3).

use super::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Typed key in [`AssocTable`].
///
/// Key ordering matches IEEE 1800-2023 §7.8:
/// - integral keys compare numerically (signed or unsigned per the declared index type);
/// - wildcard `[*]` keys compare as unsigned arbitrary-width integers, normalized so
///   numerically equal values (`128'h41` vs `32'h41`) share the same canonical key;
/// - string keys compare lexicographically;
/// - unpacked-struct keys compare lexicographically by members in declaration order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum AssocKey {
    Int64(i64),
    UInt64(u64),
    /// Normalized little-endian 64-bit words, trailing zeros stripped (`len >= 2`).
    WideUnsigned(Vec<u64>),
    /// Two's-complement signed integer wider than 64 bits (`words.len() == ceil(width/64)`).
    WideSigned { width: u32, words: Vec<u64> },
    Str(String),
    Struct(Vec<AssocKey>),
}

impl AssocKey {
    fn variant_rank(&self) -> u8 {
        match self {
            Self::Int64(_) | Self::WideSigned { .. } => 0,
            Self::UInt64(_) | Self::WideUnsigned(_) => 1,
            Self::Str(_) => 2,
            Self::Struct(_) => 3,
        }
    }

    /// Construct an unsigned key, normalizing values <= u64::MAX into `UInt64`.
    pub(super) fn from_unsigned_words(mut words: Vec<u64>) -> Self {
        while words.last() == Some(&0) {
            words.pop();
        }
        match words.as_slice() {
            [] => Self::UInt64(0),
            &[w0] => Self::UInt64(w0),
            _ => Self::WideUnsigned(words),
        }
    }

    /// Construct a signed key of exact bit width `width`.
    pub(super) fn from_signed_words(width: u32, words: Vec<u64>) -> Self {
        let w = width.max(1);
        if w <= 64 {
            let raw = words.first().copied().unwrap_or(0);
            let masked = if w == 64 {
                raw
            } else {
                let m = (1u64 << w) - 1;
                let sbit = 1u64 << (w - 1);
                let v = raw & m;
                if (v & sbit) != 0 { v | !m } else { v }
            };
            Self::Int64(masked as i64)
        } else {
            let need = (w as usize).div_ceil(64);
            let mut out = words;
            out.resize(need, 0);
            let top_bits = w % 64;
            if top_bits != 0 {
                let top_mask = (1u64 << top_bits) - 1;
                if let Some(last) = out.last_mut() {
                    *last &= top_mask;
                }
            }
            Self::WideSigned {
                width: w,
                words: out,
            }
        }
    }

    fn extract_words(val: &Value, width: u32) -> Vec<u64> {
        let w = width.max(1) as usize;
        if w <= 64 && let Some(u) = val.to_u64() {
            let mask = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
            return vec![u & mask];
        }
        let nwords = w.div_ceil(64);
        let mut words = vec![0u64; nwords];
        let copy_bits = w.min(val.width as usize);
        for b in 0..copy_bits {
            if val.get_bit(b) == LogicBit::One {
                words[b / 64] |= 1u64 << (b % 64);
            }
        }
        if val.is_signed && copy_bits > 0 && copy_bits < w {
            let sign = val.get_bit(copy_bits - 1) == LogicBit::One;
            if sign {
                for b in copy_bits..w {
                    words[b / 64] |= 1u64 << (b % 64);
                }
            }
        }
        words
    }

    fn is_wide_signed_neg(width: u32, words: &[u64]) -> bool {
        let bit = width.saturating_sub(1) as usize;
        let wi = bit / 64;
        words
            .get(wi)
            .is_some_and(|w| ((*w >> (bit % 64)) & 1) != 0)
    }

    fn signed_word_at(width: u32, words: &[u64], idx: usize) -> u64 {
        let neg = Self::is_wide_signed_neg(width, words);
        let fill = if neg { u64::MAX } else { 0u64 };
        let top_wi = (width.saturating_sub(1) as usize) / 64;
        match idx.cmp(&top_wi) {
            Ordering::Less => words.get(idx).copied().unwrap_or(0),
            Ordering::Equal => {
                let raw = words.get(idx).copied().unwrap_or(0);
                let top_bits = (width % 64) as usize;
                if top_bits == 0 || !neg {
                    raw
                } else {
                    raw | !((1u64 << top_bits) - 1)
                }
            }
            Ordering::Greater => fill,
        }
    }

    fn cmp_signed_parts(wa: u32, a: &[u64], wb: u32, b: &[u64]) -> Ordering {
        let neg_a = Self::is_wide_signed_neg(wa, a);
        let neg_b = Self::is_wide_signed_neg(wb, b);
        if neg_a != neg_b {
            return neg_b.cmp(&neg_a);
        }
        let nwords = wa.max(wb).div_ceil(64) as usize;
        for idx in (0..nwords).rev() {
            let aw = Self::signed_word_at(wa, a, idx);
            let bw = Self::signed_word_at(wb, b, idx);
            match aw.cmp(&bw) {
                Ordering::Equal => {}
                ord => return ord,
            }
        }
        Ordering::Equal
    }

    pub(super) fn to_value(&self, width: u32, is_signed: bool) -> Value {
        let target_w = width.max(1);
        match self {
            Self::Int64(i) => {
                let mut v = if target_w <= 64 {
                    Value::from_u64(*i as u64, target_w)
                } else {
                    let mut out = Value::from_u64(*i as u64, 64);
                    out.is_signed = true;
                    out.resize(target_w)
                };
                v.is_signed = is_signed;
                v
            }
            Self::UInt64(u) => {
                let mut v = if target_w <= 64 {
                    Value::from_u64(*u, target_w)
                } else {
                    Value::from_u64(*u, 64).resize(target_w)
                };
                v.is_signed = is_signed;
                v
            }
            Self::WideUnsigned(words) => {
                let mut v = Value::zero(target_w);
                let limit = target_w as usize;
                for (wi, &w) in words.iter().enumerate() {
                    if w == 0 {
                        continue;
                    }
                    let base = wi * 64;
                    if base >= limit {
                        break;
                    }
                    for b in 0..64.min(limit - base) {
                        if (w >> b) & 1 != 0 {
                            v.set_bit(base + b, LogicBit::One);
                        }
                    }
                }
                v.is_signed = is_signed;
                v
            }
            Self::WideSigned { width: kw, words } => {
                let mut v = Value::zero(target_w);
                let neg = Self::is_wide_signed_neg(*kw, words);
                for bit in 0..(target_w as usize) {
                    let wi = bit / 64;
                    let word = Self::signed_word_at(*kw, words, wi);
                    if (word >> (bit % 64)) & 1 != 0 {
                        v.set_bit(bit, LogicBit::One);
                    }
                }
                if target_w <= 64 && neg {
                    v.is_signed = is_signed;
                } else {
                    v.is_signed = is_signed;
                }
                v
            }
            Self::Str(s) => Value::from_string(s),
            Self::Struct(_) => Value::zero(target_w),
        }
    }

    pub(super) fn to_canon_string(&self) -> String {
        match self {
            Self::Int64(v) => v.to_string(),
            Self::UInt64(v) => v.to_string(),
            Self::WideUnsigned(words) => {
                let mut s = String::with_capacity(words.len() * 16);
                for (i, w) in words.iter().rev().enumerate() {
                    if i == 0 {
                        use std::fmt::Write;
                        let _ = write!(s, "{w:x}");
                    } else {
                        use std::fmt::Write;
                        let _ = write!(s, "{w:016x}");
                    }
                }
                s
            }
            Self::WideSigned { width, words } => {
                let mut hex = String::with_capacity((*width as usize).div_ceil(4) + 2);
                let top_nibbles = ((*width as usize).div_ceil(4)).max(1);
                let mut digits = Vec::with_capacity(top_nibbles);
                for nib in 0..top_nibbles {
                    let bit = nib * 4;
                    let wi = bit / 64;
                    let shift = bit % 64;
                    let val = (words.get(wi).copied().unwrap_or(0) >> shift) & 0xf;
                    digits.push(char::from_digit(val as u32, 16).unwrap_or('0'));
                }
                hex.push('s');
                for c in digits.into_iter().rev() {
                    hex.push(c);
                }
                hex
            }
            Self::Str(s) => encode_name_key(s).into_owned(),
            Self::Struct(fields) => {
                let mut out = String::from("{");
                for (i, f) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&f.to_canon_string());
                }
                out.push('}');
                out
            }
        }
    }
}

impl Ord for AssocKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Int64(a), Self::Int64(b)) => a.cmp(b),
            (Self::Int64(a), Self::WideSigned { width, words }) => {
                let aw = [*a as u64];
                Self::cmp_signed_parts(64, &aw, *width, words)
            }
            (Self::WideSigned { width, words }, Self::Int64(b)) => {
                let bw = [*b as u64];
                Self::cmp_signed_parts(*width, words, 64, &bw)
            }
            (
                Self::WideSigned {
                    width: wa,
                    words: a,
                },
                Self::WideSigned {
                    width: wb,
                    words: b,
                },
            ) => Self::cmp_signed_parts(*wa, a, *wb, b),
            (Self::UInt64(a), Self::UInt64(b)) => a.cmp(b),
            (Self::UInt64(_), Self::WideUnsigned(_)) => Ordering::Less,
            (Self::WideUnsigned(_), Self::UInt64(_)) => Ordering::Greater,
            (Self::WideUnsigned(a), Self::WideUnsigned(b)) => {
                a.len().cmp(&b.len()).then_with(|| {
                    for i in (0..a.len()).rev() {
                        match a[i].cmp(&b[i]) {
                            Ordering::Equal => {}
                            ord => return ord,
                        }
                    }
                    Ordering::Equal
                })
            }
            (Self::Str(a), Self::Str(b)) => a.cmp(b),
            (Self::Struct(a), Self::Struct(b)) => a.cmp(b),
            (Self::Int64(a), Self::UInt64(b)) => (*a as i128).cmp(&(*b as i128)),
            (Self::UInt64(a), Self::Int64(b)) => (*a as i128).cmp(&(*b as i128)),
            _ => self.variant_rank().cmp(&other.variant_rank()),
        }
    }
}

impl PartialOrd for AssocKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum AssocEntry {
    Scalar(Value),
    Compound(String),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct AssocTable {
    pub entries: BTreeMap<AssocKey, AssocEntry>,
    pub evaluated_default: Option<Value>,
}

impl Simulator {
    /// Whether associative array `name` was declared with the wildcard index `[*]`.
    pub(super) fn is_wildcard_assoc_array(&self, name: &str) -> bool {
        if self.module.wildcard_assoc_arrays.contains(name) {
            return true;
        }
        if !name.contains('#') && !name.contains("::") {
            if let Some(Some(ctx)) = self.class_context_stack.last() {
                let mut cur = Some(ctx.clone());
                while let Some(c) = cur {
                    if let Some(cd) = self.module.classes.get(&c) {
                        if cd.static_wildcard_assoc_props.contains(name) {
                            return true;
                        }
                        cur = cd.extends.clone();
                    } else {
                        break;
                    }
                }
            }
        }
        if let Some((head, member)) = name.split_once('#')
            && let Some((_, member2)) = member.rsplit_once("::")
            && let Some(cls) = head.split_once('#').map(|(c, _)| c).or(Some(head))
        {
            let mut cur = Some(cls.to_string());
            while let Some(c) = cur {
                if let Some(cd) = self.module.classes.get(&c) {
                    if cd.static_wildcard_assoc_props.contains(member2) {
                        return true;
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        if let Some(pos) = name.find('#')
            && let Ok(handle) = name[..pos].parse::<usize>()
        {
            let member = name[pos + 1..].split('[').next().unwrap_or(&name[pos + 1..]);
            let mut cur = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone());
            while let Some(c) = cur {
                if let Some(cd) = self.module.classes.get(&c) {
                    if cd.wildcard_assoc_props.contains(member) {
                        return true;
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        false
    }

    /// Declared unpacked-struct key type for associative array `name` (§7.8.5).
    pub(super) fn assoc_struct_key_type(&self, name: &str) -> Option<DataType> {
        if let Some(dt) = self.module.assoc_key_struct_types.get(name) {
            return Some(dt.clone());
        }
        if !name.contains('#') && !name.contains("::") {
            if let Some(Some(ctx)) = self.class_context_stack.last() {
                let mut cur = Some(ctx.clone());
                while let Some(c) = cur {
                    if let Some(cd) = self.module.classes.get(&c) {
                        if let Some(dt) = cd.static_assoc_key_struct_props.get(name) {
                            return Some(dt.clone());
                        }
                        cur = cd.extends.clone();
                    } else {
                        break;
                    }
                }
            }
        }
        if let Some((head, member)) = name.split_once('#')
            && let Some((_, member2)) = member.rsplit_once("::")
            && let Some(cls) = head.split_once('#').map(|(c, _)| c).or(Some(head))
        {
            let mut cur = Some(cls.to_string());
            while let Some(c) = cur {
                if let Some(cd) = self.module.classes.get(&c) {
                    if let Some(dt) = cd.static_assoc_key_struct_props.get(member2) {
                        return Some(dt.clone());
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        if let Some(pos) = name.find('#')
            && let Ok(handle) = name[..pos].parse::<usize>()
        {
            let member = name[pos + 1..].split('[').next().unwrap_or(&name[pos + 1..]);
            let mut cur = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone());
            while let Some(c) = cur {
                if let Some(cd) = self.module.classes.get(&c) {
                    if let Some(dt) = cd.assoc_key_struct_props.get(member) {
                        return Some(dt.clone());
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        None
    }

    /// Whether keys of associative array `name` are class handles (for GC tracing).
    pub(super) fn assoc_key_is_class(&self, name: &str) -> bool {
        if let Some(kn) = self.module.assoc_key_type_names.get(name) {
            return self.module.classes.contains_key(kn.as_str());
        }
        if let Some(pos) = name.find('#')
            && let Ok(handle) = name[..pos].parse::<usize>()
        {
            let member = name[pos + 1..].split('[').next().unwrap_or(&name[pos + 1..]);
            let mut cur = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone());
            while let Some(c) = cur {
                if let Some(cd) = self.module.classes.get(&c) {
                    if let Some(kn) = cd.assoc_key_types.get(member) {
                        return self.module.classes.contains_key(kn.as_str());
                    }
                    cur = cd.extends.clone();
                } else {
                    break;
                }
            }
        }
        false
    }

    fn assoc_key_from_dt_val(&self, dt: &DataType, val: &Value) -> Option<AssocKey> {
        let resolved = self.resolve_dt_ref(dt);
        if matches!(
            resolved,
            DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        ) {
            return Some(AssocKey::Str(val.to_sv_string()));
        }
        if val.has_xz() {
            return None;
        }
        let w = super::elaborate::resolve_type_width(
            resolved,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        )
        .max(1);
        let is_signed =
            super::elaborate::is_type_signed_resolved(resolved, &self.module.typedef_types);
        let fitted = val.resize_for_assign(w);
        let words = AssocKey::extract_words(&fitted, w);
        if is_signed {
            Some(AssocKey::from_signed_words(w, words))
        } else {
            Some(AssocKey::from_unsigned_words(words))
        }
    }

    fn eval_assoc_struct_key(&mut self, expr: &Expression, dt: &DataType) -> Option<AssocKey> {
        let resolved = self.resolve_dt_ref(dt).clone();
        let DataType::Struct(su) = resolved else {
            let v = self.eval_expr(expr);
            return self.assoc_key_from_dt_val(dt, &v);
        };
        if su.packed {
            let v = self.eval_expr(expr);
            return self.assoc_key_from_dt_val(dt, &v);
        }
        let mut fields = Vec::new();
        let unwrapped = Self::peel_pattern_parens(expr);
        if let ExprKind::AssignmentPattern(items) = &unwrapped.kind {
            let ordered = self.pattern_ordered(items);
            let mut idx = 0usize;
            for m in &su.members {
                for decl in &m.declarators {
                    let field_expr = ordered.get(idx).copied().or_else(|| {
                        items.iter().find_map(|it| match it {
                            AssignmentPatternItem::Named(nk, expr)
                                if nk.ident.name == decl.name.name =>
                            {
                                Some(expr)
                            }
                            AssignmentPatternItem::Default(e) => Some(e),
                            _ => None,
                        })
                    });
                    idx += 1;
                    let Some(fe) = field_expr else {
                        return None;
                    };
                    let fk = if matches!(
                        self.resolve_dt_ref(&m.data_type),
                        DataType::Struct(sub) if !sub.packed
                    ) {
                        self.eval_assoc_struct_key(fe, &m.data_type)?
                    } else {
                        let fv = self.eval_expr(fe);
                        self.assoc_key_from_dt_val(&m.data_type, &fv)?
                    };
                    fields.push(fk);
                }
            }
            return Some(AssocKey::Struct(fields));
        }

        // Read member-by-member using synthesized `MemberAccess` nodes so local,
        // module, array-element, and class-property structs all resolve uniformly.
        for m in &su.members {
            for decl in &m.declarators {
                let member_expr = Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(expr.clone()),
                        member: decl.name.clone(),
                    },
                    expr.span,
                );
                let fk = if matches!(
                    self.resolve_dt_ref(&m.data_type),
                    DataType::Struct(sub) if !sub.packed
                ) {
                    self.eval_assoc_struct_key(&member_expr, &m.data_type)?
                } else {
                    let fv = self.eval_expr(&member_expr);
                    self.assoc_key_from_dt_val(&m.data_type, &fv)?
                };
                fields.push(fk);
            }
        }
        Some(AssocKey::Struct(fields))
    }

    fn peel_pattern_parens(mut expr: &Expression) -> &Expression {
        while let ExprKind::Paren(inner) = &expr.kind {
            expr = inner.as_ref();
        }
        expr
    }

    /// Resolve `base` to its declared unpacked-struct associative key type, if any.
    pub(super) fn assoc_struct_key_type_for_expr(&mut self, base: &Expression) -> Option<DataType> {
        let an = self
            .expr_assoc_name(base)
            .or_else(|| self.plain_assoc_operand(base))?;
        self.assoc_struct_key_type(&an)
    }

    fn assoc_key_to_expr(key: &AssocKey, span: crate::ast::Span) -> Expression {
        let kind = match key {
            AssocKey::Struct(fields) => ExprKind::AssignmentPattern(
                fields
                    .iter()
                    .map(|f| AssignmentPatternItem::Ordered(Self::assoc_key_to_expr(f, span)))
                    .collect(),
            ),
            AssocKey::Str(s) => ExprKind::StringLiteral(s.clone()),
            AssocKey::Int64(v) => {
                let raw = *v as u64;
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(64),
                    signed: true,
                    base: NumberBase::Binary,
                    value: Value::from_u64(raw, 64).to_bin(),
                    cached_val: crate::ast::cache::LiteralCache::new(Some((raw, 0u64, 64u32))),
                })
            }
            AssocKey::UInt64(v) => ExprKind::Number(NumberLiteral::Integer {
                size: Some(64),
                signed: false,
                base: NumberBase::Binary,
                value: Value::from_u64(*v, 64).to_bin(),
                cached_val: crate::ast::cache::LiteralCache::new(Some((*v, 0u64, 64u32))),
            }),
            AssocKey::WideUnsigned(words) => {
                let w = (words.len() as u32 * 64).max(65);
                let val = key.to_value(w, false);
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(w),
                    signed: false,
                    base: NumberBase::Binary,
                    value: val.to_bin(),
                    cached_val: crate::ast::cache::LiteralCache::new(None),
                })
            }
            AssocKey::WideSigned { width, .. } => {
                let w = (*width).max(1);
                let val = key.to_value(w, true);
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(w),
                    signed: true,
                    base: NumberBase::Binary,
                    value: val.to_bin(),
                    cached_val: crate::ast::cache::LiteralCache::new(None),
                })
            }
        };
        Expression::new(kind, span)
    }

    /// Evaluate an unpacked-struct AA key expression once and freeze it as a canonical
    /// pattern literal so side effects and NBA schedule-time values are preserved (§7.8.5).
    pub(super) fn freeze_struct_key_expr(
        &mut self,
        expr: &Expression,
        dt: &DataType,
    ) -> Option<Expression> {
        let key = self.eval_assoc_struct_key(expr, dt)?;
        Some(Self::assoc_key_to_expr(&key, expr.span))
    }

    /// Evaluate `index` as a typed [`AssocKey`] for associative array `name`.
    /// Returns `None` if an integral index (or integral struct key field) has X/Z bits (§7.8.1/§7.8.6).
    pub(super) fn eval_assoc_key(&mut self, name: &str, index: &Expression) -> Option<AssocKey> {
        if let Some(dt) = self.assoc_struct_key_type(name) {
            return self.eval_assoc_struct_key(index, &dt);
        }
        let iv = self.eval_expr(index);
        self.assoc_key_from_val(name, &iv)
    }

    /// Convert an already-evaluated index [`Value`] into a typed [`AssocKey`].
    /// Returns `None` for non-string keys containing X/Z bits (§7.8).
    pub(super) fn assoc_key_from_val(&self, name: &str, val: &Value) -> Option<AssocKey> {
        if self.is_string_keyed_array(name) {
            return Some(AssocKey::Str(val.to_sv_string()));
        }
        if val.has_xz() {
            return None;
        }
        if self.is_wildcard_assoc_array(name) {
            // §7.8.1: wildcard `[*]` treats integral indices as unsigned regardless
            // of operand signedness and compares arbitrarily wide indices numerically.
            let words = AssocKey::extract_words(val, val.width.max(1));
            return Some(AssocKey::from_unsigned_words(words));
        }
        if let Some((kw, is_signed)) = self.assoc_index_width_for(name) {
            // §7.8.4: integral index converts to declared key type width + signedness.
            let fitted = val.resize_for_assign(kw.max(1));
            let words = AssocKey::extract_words(&fitted, kw.max(1));
            if is_signed {
                return Some(AssocKey::from_signed_words(kw, words));
            }
            return Some(AssocKey::from_unsigned_words(words));
        }
        // Fallback for class-handle or unresolved untyped index.
        let words = AssocKey::extract_words(val, val.width.max(1));
        if val.is_signed && val.width <= 64 {
            Some(AssocKey::from_signed_words(val.width, words))
        } else {
            Some(AssocKey::from_unsigned_words(words))
        }
    }

    /// Whether the element type of associative array `name` is 2-state.
    pub(super) fn is_assoc_elem_two_state(&self, name: &str) -> bool {
        if self.module.two_state_signals.contains(name) {
            return true;
        }
        if let Some(dt) = self.assoc_elem_decl_type(name) {
            return super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types);
        }
        if let Some(pos) = name.find('#')
            && let Ok(handle) = name[..pos].parse::<usize>()
        {
            let member = name[pos + 1..].split('[').next().unwrap_or(&name[pos + 1..]);
            return self.class_prop_two_state(handle, member);
        }
        false
    }

    /// Fit `val` to the declared element width, 2-state value set, and signedness of AA `name`.
    pub(super) fn fit_assoc_elem_val(&self, name: &str, val: Value) -> Value {
        if val.is_real || self.is_string_collection(name) {
            return val;
        }
        let mut fitted = match self.assoc_elem_width(name) {
            Some(w) if (w != val.width || val.is_fill) && w > 0 => val.resize_for_assign(w),
            _ => val,
        };
        if fitted.has_xz() && self.is_assoc_elem_two_state(name) {
            fitted = fitted.to_two_state();
        }
        if let Some(sg) = self.assoc_elem_signed(name) {
            fitted.is_signed = sg;
        } else if !fitted.is_signed && self.signed_class_coll(name) {
            fitted = self.fit_signed_class_coll_elem(name, fitted);
        }
        fitted
    }

    /// Read a scalar element from `assoc_tables[name]`.
    pub(super) fn assoc_get_scalar(&self, name: &str, key: &AssocKey) -> Option<Value> {
        let entry = self.assoc_tables.get(name)?.entries.get(key)?;
        match entry {
            AssocEntry::Scalar(v) => Some(v.clone()),
            AssocEntry::Compound(elem) => self.get_signal_value_by_name(elem),
        }
    }

    /// Compatibility lookup by canonical key string (`key.to_canon_string()`).
    pub(super) fn assoc_get_by_canon(&self, name: &str, canon: &str) -> Option<Value> {
        if let Some(table) = self.assoc_tables.get(name) {
            if let Some(val) = table
                .entries
                .iter()
                .find(|(k, _)| k.to_canon_string() == canon)
                .and_then(|(_, e)| match e {
                    AssocEntry::Scalar(v) => Some(v.clone()),
                    AssocEntry::Compound(elem) => self.get_signal_value_by_name(elem),
                })
            {
                return Some(val);
            }
        }
        let elem = Self::name_with_key(name, canon);
        self.get_signal_value_by_name(&elem)
    }

    /// Parse a canonical key string back into an `AssocKey` for AA `name`.
    pub(super) fn assoc_key_from_canon(&self, name: &str, canon: &str) -> Option<AssocKey> {
        if self.is_string_keyed_array(name) {
            return Some(AssocKey::Str(decode_name_key(canon).into_owned()));
        }
        if self.is_wildcard_assoc_array(name) {
            if let Ok(u) = canon.parse::<u64>() {
                return Some(AssocKey::UInt64(u));
            }
            if canon.chars().all(|c| c.is_ascii_hexdigit()) {
                let w = (canon.len() as u32 * 4).max(65);
                let v = Value::from_str_radix(canon, 16, w);
                return Some(AssocKey::from_unsigned_words(AssocKey::extract_words(&v, w)));
            }
            return None;
        }
        if let Some((kw, is_signed)) = self.assoc_index_width_for(name) {
            if kw > 64 {
                if is_signed {
                    let hex = canon.strip_prefix('s').unwrap_or(canon);
                    let v = Value::from_str_radix(hex, 16, kw);
                    return Some(AssocKey::from_signed_words(kw, AssocKey::extract_words(&v, kw)));
                }
                let v = Value::from_str_radix(canon, 16, kw);
                return Some(AssocKey::from_unsigned_words(AssocKey::extract_words(&v, kw)));
            }
            if is_signed {
                return canon.parse::<i64>().ok().map(AssocKey::Int64);
            }
            return canon.parse::<u64>().ok().map(AssocKey::UInt64);
        }
        if let Ok(i) = canon.parse::<i64>() {
            return Some(AssocKey::Int64(i));
        }
        if let Ok(u) = canon.parse::<u64>() {
            return Some(AssocKey::UInt64(u));
        }
        Some(AssocKey::Str(decode_name_key(canon).into_owned()))
    }

    /// Insert or update an element in `assoc_tables[name]` addressed by canonical key string.
    pub(super) fn assoc_set_by_canon(&mut self, name: &str, canon: &str, val: Value) -> bool {
        let existing_key = self
            .assoc_tables
            .get(name)
            .and_then(|t| t.entries.keys().find(|k| k.to_canon_string() == canon).cloned());
        let Some(key) = existing_key.or_else(|| self.assoc_key_from_canon(name, canon)) else {
            return false;
        };
        let fitted = self.fit_assoc_elem_val(name, val);
        self.assoc_set_scalar(name, key, fitted)
    }

    fn mark_parent_assoc_compound(&mut self, child_name: &str) {
        let mut cur = child_name.to_string();
        for _ in 0..16 {
            let Some((parent, rest)) = cur.rsplit_once('[') else {
                break;
            };
            if !rest.ends_with(']') || parent.is_empty() {
                break;
            }
            let canon = &rest[..rest.len() - 1];
            if !self.is_associative_array(parent) && !self.assoc_tables.contains_key(parent) {
                break;
            }
            let Some(pkey) = self.assoc_key_from_canon(parent, canon) else {
                break;
            };
            let parent_was_empty = self
                .assoc_tables
                .get(parent)
                .is_none_or(|t| t.entries.is_empty());
            let table = self.assoc_tables.entry(parent.to_string()).or_default();
            table
                .entries
                .entry(pkey)
                .or_insert_with(|| AssocEntry::Compound(cur.clone()));
            if !parent_was_empty {
                break;
            }
            cur = parent.to_string();
        }
    }

    /// Insert or update a scalar AA element in `assoc_tables[name]`. Returns whether changed.
    pub(super) fn assoc_set_scalar(&mut self, name: &str, key: AssocKey, val: Value) -> bool {
        let was_empty = self
            .assoc_tables
            .get(name)
            .is_none_or(|t| t.entries.is_empty());
        let table = self.assoc_tables.entry(name.to_string()).or_default();
        let changed = table.entries.get(&key) != Some(&AssocEntry::Scalar(val.clone()));
        table.entries.insert(key, AssocEntry::Scalar(val));
        if was_empty && (self.assoc_elem_colls.contains(name) || name.contains("].")) {
            self.mark_struct_coll_owner_present(name);
        }
        if was_empty && name.ends_with(']') {
            self.mark_parent_assoc_compound(name);
        }
        changed
    }

    /// Record a compound AA element (`struct` / nested collection) whose children live under `prefix`.
    pub(super) fn assoc_mark_compound(&mut self, name: &str, key: AssocKey, prefix: String) {
        let was_empty = self
            .assoc_tables
            .get(name)
            .is_none_or(|t| t.entries.is_empty());
        let table = self.assoc_tables.entry(name.to_string()).or_default();
        table
            .entries
            .entry(key)
            .or_insert(AssocEntry::Compound(prefix));
        if was_empty && name.ends_with(']') {
            self.mark_parent_assoc_compound(name);
        }
    }

    fn assoc_delete_compound_prefix(&mut self, prefix: &str) {
        self.signals.remove(prefix);
        let dot = Self::name_with_suffix(prefix, ".");
        let brk = Self::name_with_suffix(prefix, "[");
        for k in self.signals.keys_with_prefix(&dot) {
            self.signals.remove(&k);
        }
        for k in self.signals.keys_with_elem_prefix(&brk) {
            self.signals.remove(&k);
        }
        let sz = Self::name_with_suffix(prefix, ".size");
        self.signals.remove(&sz);
        if self.module.dynamic_arrays.contains(prefix) || self.module.queue_vars.contains(prefix) {
            self.signals.truncate_sequence(prefix, 0);
        }
        self.module.dynamic_arrays.remove(prefix);
        self.module.queue_vars.remove(prefix);
        self.module.queue_max_sizes.remove(prefix);
        self.module.arrays.remove(prefix);
        self.module.associative_arrays.remove(prefix);
        self.assoc_tables.remove(prefix);
        self.assoc_tables
            .retain(|k, _| !k.starts_with(&brk) && !k.starts_with(&dot));
        self.module
            .dynamic_arrays
            .retain(|k| !k.starts_with(&brk) && !k.starts_with(&dot));
        self.module
            .queue_vars
            .retain(|k| !k.starts_with(&brk) && !k.starts_with(&dot));
        self.module
            .queue_max_sizes
            .retain(|k, _| !k.starts_with(&brk) && !k.starts_with(&dot));
        if self.queue_elem_ids.contains_key(prefix) {
            self.detach_all_qrefs(prefix);
        }
    }

    /// Delete one key from associative array `name`. Returns `true` if removed.
    pub(super) fn assoc_delete_key(&mut self, name: &str, key: &AssocKey) -> bool {
        let removed = self
            .assoc_tables
            .get_mut(name)
            .and_then(|t| t.entries.remove(key));
        let canon = key.to_canon_string();
        let legacy_key = Self::name_with_key(name, &canon);
        let had_legacy = self.signals.remove(&legacy_key).is_some()
            || self.module.dynamic_arrays.contains(&legacy_key)
            || self.assoc_tables.contains_key(&legacy_key)
            || self
                .signals
                .any_key_with_prefix(&Self::name_with_suffix(&legacy_key, "."))
            || self
                .signals
                .any_key_with_prefix(&Self::name_with_suffix(&legacy_key, "["));
        if let Some(AssocEntry::Compound(prefix)) = &removed {
            self.assoc_delete_compound_prefix(prefix);
        } else if had_legacy {
            self.assoc_delete_compound_prefix(&legacy_key);
        }
        removed.is_some() || had_legacy
    }

    /// Clear all entries of associative array `name`.
    /// When `clear_default` is `true` (e.g. `aa = '{...}` overwrite, §7.9.11),
    /// also clears `evaluated_default`, `module.assoc_defaults`, and `class_assoc_defaults`.
    /// When `clear_default` is `false` (`aa.delete()`), keeps `evaluated_default` per SV semantics.
    pub(super) fn assoc_clear(&mut self, name: &str, clear_default: bool) {
        if let Some(table) = self.assoc_tables.get_mut(name) {
            let old = std::mem::take(&mut table.entries);
            if clear_default {
                table.evaluated_default = None;
            }
            for (_, entry) in old {
                if let AssocEntry::Compound(prefix) = entry {
                    self.assoc_delete_compound_prefix(&prefix);
                }
            }
        }
        if clear_default {
            self.module.assoc_defaults.remove(name);
            self.class_assoc_defaults.remove(name);
        }
        let prefix = Self::name_with_suffix(name, "[");
        for k in self.signals.keys_with_elem_prefix(&prefix) {
            self.signals.remove(&k);
        }
        self.module.dynamic_arrays.retain(|k| !k.starts_with(&prefix));
        self.module.queue_vars.retain(|k| !k.starts_with(&prefix));
        self.module.queue_max_sizes.retain(|k, _| !k.starts_with(&prefix));
        self.module
            .associative_arrays
            .retain(|k, _| !k.starts_with(&prefix));
        self.assoc_tables.retain(|k, _| !k.starts_with(&prefix));
        if !self.queue_elem_ids.is_empty() {
            let affected: Vec<String> = self
                .queue_elem_ids
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .cloned()
                .collect();
            for q in affected {
                self.detach_all_qrefs(&q);
            }
        }
    }

    /// Number of populated top-level keys in associative array `name` (`O(1)` via `BTreeMap`).
    pub(super) fn assoc_len(&self, name: &str) -> usize {
        if let Some(t) = self.assoc_tables.get(name) {
            if !t.entries.is_empty() {
                return t.entries.len();
            }
        }
        let prefix = Self::name_with_suffix(name, "[");
        let c_static = self.assoc_static_keys(&prefix).len();
        if c_static == 0 {
            let mut live_count = 0usize;
            let mut compound = false;
            for k in self.signals.elem_keys_with_prefix(&prefix) {
                live_count += 1;
                let rest = &k[prefix.len()..];
                if rest.contains("][") || rest.contains('.') {
                    compound = true;
                }
            }
            if !compound {
                return live_count;
            }
        }
        self.assoc_top_level_keys_shared(name).len()
    }

    /// Evaluate (once) and return any explicit `default:` value on associative array `name` (§7.9.11).
    pub(super) fn assoc_default_val(&mut self, name: &str) -> Option<Value> {
        if let Some(v) = self
            .assoc_tables
            .get(name)
            .and_then(|t| t.evaluated_default.clone())
        {
            return Some(v);
        }
        if !self.class_assoc_defaults.is_empty()
            && let Some(v) = self.class_assoc_defaults.get(name).cloned()
        {
            let fitted = self.fit_assoc_elem_val(name, v);
            self.assoc_tables
                .entry(name.to_string())
                .or_default()
                .evaluated_default = Some(fitted.clone());
            return Some(fitted);
        }
        if let Some(def_expr) = self.module.assoc_defaults.get(name).cloned() {
            let v = match self.assoc_elem_width(name) {
                Some(w) if w > 0 => self.eval_expr_ctx(&def_expr, w),
                _ => self.eval_expr(&def_expr),
            };
            let fitted = self.fit_assoc_elem_val(name, v);
            self.assoc_tables
                .entry(name.to_string())
                .or_default()
                .evaluated_default = Some(fitted.clone());
            return Some(fitted);
        }
        None
    }

    /// Write an [`AssocKey`] into an lvalue expression `var` (used by `first`/`last`/`next`/`prev`).
    pub(super) fn write_assoc_key_var(
        &mut self,
        var: &Expression,
        key: &AssocKey,
        key_type: Option<(u32, bool)>,
        struct_dt: Option<&DataType>,
    ) {
        if let AssocKey::Struct(fields) = key
            && let Some(dt) = struct_dt
        {
            let resolved = self.resolve_dt_ref(dt).clone();
            if let DataType::Struct(su) = resolved {
                let mut fidx = 0usize;
                for m in &su.members {
                    for decl in &m.declarators {
                        if let Some(fk) = fields.get(fidx) {
                            let member_lval = Expression::new(
                                ExprKind::MemberAccess {
                                    expr: Box::new(var.clone()),
                                    member: decl.name.clone(),
                                },
                                var.span,
                            );
                            let fw = super::elaborate::resolve_type_width(
                                &m.data_type,
                                Some(&self.module.parameters),
                                Some(&self.module.typedefs),
                            )
                            .max(1);
                            let fsigned = super::elaborate::is_type_signed_resolved(
                                &m.data_type,
                                &self.module.typedef_types,
                            );
                            self.write_assoc_key_var(
                                &member_lval,
                                fk,
                                Some((fw, fsigned)),
                                Some(&m.data_type),
                            );
                        }
                        fidx += 1;
                    }
                }
                return;
            }
        }
        let arg_w = self.eval_lvalue_width(var).max(1);
        let (_, ks) = key_type.unwrap_or((arg_w, matches!(key, AssocKey::Int64(_) | AssocKey::WideSigned { .. })));
        let val = key.to_value(arg_w, ks);
        self.assign_value(var, &val);
    }

    /// Accurate bit width of a key variable lvalue (checking local frame, signal table, and widths map).
    pub(super) fn eval_lvalue_width(&mut self, expr: &Expression) -> u32 {
        let peeled = Self::peel_pattern_parens(expr);
        if let ExprKind::Ident(h) = &peeled.kind {
            if h.path.len() == 1 {
                let bare = &h.path[0].name.name;
                if let Some(v) = self.local_stack.last().and_then(|f| f.get(bare)) {
                    if v.width > 0 {
                        return v.width;
                    }
                }
            }
            let n = self.resolve_hier_name(h);
            if let Some(w) = self.lookup_signal_width(&n) {
                return w;
            }
            if let Some(&w) = self.widths.get(n.as_ref()) {
                return w;
            }
            if let Some(v) = self.get_signal_value_by_name(&n) {
                return v.width;
            }
        }
        self.infer_width(peeled)
    }

    fn collect_struct_string_leaves(
        &self,
        prefix: &str,
        su: &crate::ast::types::StructUnionType,
        out: &mut HashSet<String>,
    ) {
        for m in &su.members {
            let resolved = self.resolve_dt_ref(&m.data_type);
            for decl in &m.declarators {
                if !decl.dimensions.is_empty() {
                    continue;
                }
                let leaf = format!("{prefix}.{}", decl.name.name);
                match resolved {
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    } => {
                        out.insert(leaf);
                    }
                    DataType::Struct(sub) if !sub.packed => {
                        self.collect_struct_string_leaves(&leaf, sub, out);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Ensure an unpacked-struct element `elem_prefix` at `key` in `base` has its declared
    /// member defaults (or type defaults) initialized before updating one member (§7.8.7).
    pub(super) fn init_assoc_struct_elem_defaults(
        &mut self,
        base: &str,
        key: AssocKey,
        elem_prefix: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        let already_exists = self
            .assoc_tables
            .get(base)
            .is_some_and(|t| t.entries.contains_key(&key))
            || self.signals.contains_key(elem_prefix);
        let mut string_leaves = HashSet::default();
        self.collect_struct_string_leaves(elem_prefix, su, &mut string_leaves);
        for sk in &string_leaves {
            self.string_signals.insert(sk.clone());
        }
        if !already_exists {
            let seeds = self.struct_return_leaf_seeds(elem_prefix, su);
            for (leaf_key, leaf_val) in seeds {
                if self.signals.get(&leaf_key).is_none() {
                    let seed = if string_leaves.contains(&leaf_key) {
                        Value::from_string("")
                    } else {
                        leaf_val
                    };
                    self.signals.insert(leaf_key, seed);
                }
            }
            for sk in &string_leaves {
                if self.signals.get(sk).is_none() {
                    self.signals.insert(sk.clone(), Value::from_string(""));
                }
            }
            if self.signals.get(elem_prefix).is_none() {
                self.signals
                    .insert(elem_prefix.to_string(), Value::zero(1));
            }
        }
        self.assoc_mark_compound(base, key, elem_prefix.to_string());
    }

    /// Resolve a struct-member write receiver `b[k]` to `(elem_prefix, Some((base, key)), su)` for
    /// both class-property and module/local unpacked-struct associative arrays (§7.8.7).
    pub(super) fn resolve_assoc_struct_elem(
        &mut self,
        element: &Expression,
    ) -> Option<(String, Option<(String, AssocKey)>, crate::ast::types::StructUnionType)> {
        if let Some((elem, su)) = self.coll_assoc_struct_elem(element) {
            let top = if let ExprKind::Index { expr: base, index } = &element.kind
                && let Some(an) = self
                    .expr_assoc_name(base)
                    .or_else(|| self.plain_assoc_operand(base))
                && self.is_associative_array(&an)
                && !self.is_associative_array(&elem)
                && let Some(key) = self.eval_assoc_key(&an, index)
            {
                let canon_elem = Self::name_with_key(&an, &key.to_canon_string());
                return Some((canon_elem, Some((an, key)), su));
            } else {
                None
            };
            return Some((elem, top, su));
        }
        let ExprKind::Index { expr: base, index } = &element.kind else {
            return None;
        };
        let base_name = self
            .expr_assoc_name(base)
            .or_else(|| self.plain_assoc_operand(base))?;
        if !self.is_associative_array(&base_name) {
            return None;
        }
        let su = self.queue_elem_struct(&base_name)?;
        let key = self.eval_assoc_key(&base_name, index)?;
        let elem_prefix = Self::name_with_key(&base_name, &key.to_canon_string());
        Some((elem_prefix, Some((base_name, key)), su))
    }

    /// Lazily allocate unique per-element IDs for queue `qname` when binding a `ref` formal (§7.10.3).
    pub(super) fn ensure_queue_elem_ids(&mut self, qname: &str, size: usize) -> &mut VecDeque<u64> {
        let ids = self
            .queue_elem_ids
            .entry(qname.to_string())
            .or_insert_with(VecDeque::new);
        while ids.len() < size {
            let id = self.next_qref_id;
            self.next_qref_id = self.next_qref_id.wrapping_add(1).max(1);
            ids.push_back(id);
        }
        while ids.len() > size {
            ids.pop_back();
        }
        ids
    }

    pub(super) fn qref_track_insert(&mut self, qname: &str, idx: usize, new_size: usize) {
        let mut dropped_ids = Vec::new();
        {
            let Some(ids) = self.queue_elem_ids.get_mut(qname) else {
                return;
            };
            let id = self.next_qref_id;
            self.next_qref_id = self.next_qref_id.wrapping_add(1).max(1);
            let pos = idx.min(ids.len());
            ids.insert(pos, id);
            while ids.len() > new_size {
                if let Some(dropped_id) = ids.pop_back() {
                    dropped_ids.push(dropped_id);
                }
            }
        }
        for dropped_id in dropped_ids {
            let v = self
                .get_signal_value_by_name(&format!("{}[{}]", qname, new_size))
                .unwrap_or_else(|| Value::zero(32));
            self.detached_qref_vals.insert(dropped_id, v);
        }
    }

    pub(super) fn qref_track_remove(&mut self, qname: &str, idx: usize, detached_val: Value) {
        let Some(ids) = self.queue_elem_ids.get_mut(qname) else {
            return;
        };
        if idx < ids.len()
            && let Some(id) = ids.remove(idx)
        {
            self.detached_qref_vals.insert(id, detached_val);
        }
    }

    pub(super) fn detach_all_qrefs(&mut self, qname: &str) {
        let Some(ids) = self.queue_elem_ids.remove(qname) else {
            return;
        };
        for (i, id) in ids.into_iter().enumerate() {
            let v = self
                .get_signal_value_by_name(&format!("{}[{}]", qname, i))
                .unwrap_or_else(|| Value::zero(32));
            self.detached_qref_vals.insert(id, v);
        }
    }
}
