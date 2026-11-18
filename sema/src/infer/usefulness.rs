use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::modules::source_type_name;
use crate::typed_ast::{TypedExpr, TypedMatchArm, TypedPattern, TypedPatternKind};
use crate::types::{InferType, TypeTable};
use aelys_syntax::{MatchOrigin, PatLit, Span};

const STEP_LIMIT: usize = 1_000_000;
const WITNESS_LIMIT: usize = 16;

#[derive(Clone, PartialEq, Debug)]
enum Ctor {
    Variant(u32),
    Single,
    Range(i128, i128),
    Str(String),
}

#[derive(Clone, Debug)]
enum Pat {
    Wild(InferType),
    Ctor(Ctor, Vec<Pat>, InferType),
    Or(Vec<Pat>),
}

#[derive(Clone, Debug)]
enum Wit {
    Wild,
    Ctor(Ctor, Vec<Wit>, InferType),
}

enum Universe {
    Variants(Vec<u32>),
    Single,
    Ranges(Vec<(i128, i128)>),
    Infinite,
}

struct Checker<'a> {
    table: &'a TypeTable,
    steps: usize,
}

struct TooComplex;

impl Pat {
    fn ty(&self) -> &InferType {
        match self {
            Pat::Wild(t) | Pat::Ctor(_, _, t) => t,
            Pat::Or(alts) => alts[0].ty(),
        }
    }
}

fn lit_value(lit: &PatLit) -> Option<i128> {
    match lit {
        PatLit::Int(v) => Some(*v),
        PatLit::Char(c) => Some(*c as i128),
        PatLit::Bool(b) => Some(*b as i128),
        PatLit::Str(_) => None,
    }
}

fn lower(p: &TypedPattern) -> Pat {
    match &p.kind {
        TypedPatternKind::Wildcard | TypedPatternKind::Binding { sub: None, .. } => {
            Pat::Wild(p.ty.clone())
        }
        TypedPatternKind::Binding { sub: Some(sub), .. } => lower(sub),
        TypedPatternKind::Lit(PatLit::Str(s)) => {
            Pat::Ctor(Ctor::Str(s.clone()), Vec::new(), p.ty.clone())
        }
        TypedPatternKind::Lit(lit) => {
            let v = lit_value(lit).unwrap_or_default();
            Pat::Ctor(Ctor::Range(v, v), Vec::new(), p.ty.clone())
        }
        TypedPatternKind::Range { lo, hi } => Pat::Ctor(
            Ctor::Range(
                lit_value(lo).unwrap_or_default(),
                lit_value(hi).unwrap_or_default(),
            ),
            Vec::new(),
            p.ty.clone(),
        ),
        TypedPatternKind::Variant { tag, fields, .. } => Pat::Ctor(
            Ctor::Variant(*tag),
            fields.iter().map(lower).collect(),
            p.ty.clone(),
        ),
        TypedPatternKind::Struct { fields, .. } => Pat::Ctor(
            Ctor::Single,
            fields.iter().map(|(_, f)| lower(f)).collect(),
            p.ty.clone(),
        ),
        TypedPatternKind::Tuple(elems) => Pat::Ctor(
            Ctor::Single,
            elems.iter().map(lower).collect(),
            p.ty.clone(),
        ),
        TypedPatternKind::Or(alts) => Pat::Or(alts.iter().map(lower).collect()),
    }
}

fn char_domain() -> Vec<(i128, i128)> {
    vec![(0, 0xD7FF), (0xE000, 0x10FFFF)]
}

impl Checker<'_> {
    fn universe(&self, ty: &InferType) -> Universe {
        match ty {
            InferType::Bool => Universe::Ranges(vec![(0, 1)]),
            InferType::Char => Universe::Ranges(char_domain()),
            t if t.is_integer() => match t.int_bounds() {
                Some(bounds) => Universe::Ranges(vec![bounds]),
                None => Universe::Infinite,
            },
            InferType::Enum(name, _) => match self.table.get_enum(name) {
                Some(def) => Universe::Variants(def.variants.iter().map(|v| v.tag).collect()),
                None => Universe::Infinite,
            },
            InferType::Struct(name, _) if self.table.get_struct(name).is_some() => Universe::Single,
            InferType::Tuple(_) => Universe::Single,
            _ => Universe::Infinite,
        }
    }

    fn step(&mut self) -> Result<(), TooComplex> {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            Err(TooComplex)
        } else {
            Ok(())
        }
    }

    fn expand_rows(rows: &[Vec<Pat>]) -> Vec<Vec<Pat>> {
        let mut out = Vec::with_capacity(rows.len());
        let mut stack: Vec<Vec<Pat>> = rows.iter().rev().cloned().collect();
        while let Some(row) = stack.pop() {
            match row.first() {
                Some(Pat::Or(alts)) => {
                    for alt in alts.iter().rev() {
                        let mut r = Vec::with_capacity(row.len());
                        r.push(alt.clone());
                        r.extend(row[1..].iter().cloned());
                        stack.push(r);
                    }
                }
                _ => out.push(row),
            }
        }
        out
    }

    fn specialize(row: &[Pat], c: &Ctor, field_tys: &[InferType]) -> Option<Vec<Pat>> {
        let head = &row[0];
        let mut out: Vec<Pat> = match head {
            Pat::Wild(_) => field_tys.iter().map(|t| Pat::Wild(t.clone())).collect(),
            Pat::Ctor(hc, args, _) => {
                let matches = match (hc, c) {
                    (Ctor::Range(a, b), Ctor::Range(x, y)) => a <= x && y <= b,
                    _ => hc == c,
                };
                if !matches {
                    return None;
                }
                args.clone()
            }
            Pat::Or(_) => return None,
        };
        out.extend(row[1..].iter().cloned());
        Some(out)
    }

    fn field_types(rows: &[Vec<Pat>], q: &[Pat], c: &Ctor) -> Vec<InferType> {
        let found = std::iter::once(q)
            .chain(rows.iter().map(|r| r.as_slice()))
            .find_map(|r| match &r[0] {
                Pat::Ctor(hc, args, _) if hc == c => {
                    Some(args.iter().map(|a| a.ty().clone()).collect())
                }
                _ => None,
            });
        found.unwrap_or_default()
    }

    fn split(ranges: &[(i128, i128)], target: &[(i128, i128)]) -> Vec<(i128, i128)> {
        let mut cuts: Vec<i128> = Vec::new();
        for (a, b) in ranges {
            cuts.push(*a);
            cuts.push(b + 1);
        }
        cuts.sort();
        cuts.dedup();
        let mut out = Vec::new();
        for (lo, hi) in target {
            let mut start = *lo;
            for c in cuts.iter().copied() {
                if c > start && c <= *hi {
                    out.push((start, c - 1));
                    start = c;
                }
            }
            out.push((start, *hi));
        }
        out
    }

    fn useful(&mut self, rows: &[Vec<Pat>], q: &[Pat]) -> Result<Vec<Vec<Wit>>, TooComplex> {
        self.step()?;
        if q.is_empty() {
            return Ok(if rows.is_empty() {
                vec![Vec::new()]
            } else {
                Vec::new()
            });
        }
        if let Pat::Or(alts) = &q[0] {
            let mut all = Vec::new();
            for alt in alts {
                let mut q2 = vec![alt.clone()];
                q2.extend(q[1..].iter().cloned());
                all.extend(self.useful(rows, &q2)?);
                if all.len() >= WITNESS_LIMIT {
                    break;
                }
            }
            return Ok(all);
        }
        let rows = Self::expand_rows(rows);
        let ty = q[0].ty().clone();
        let head_ranges: Vec<(i128, i128)> = rows
            .iter()
            .filter_map(|r| match &r[0] {
                Pat::Ctor(Ctor::Range(a, b), _, _) => Some((*a, *b)),
                _ => None,
            })
            .collect();
        if let Pat::Ctor(c, _, _) = &q[0] {
            let ctors = match c {
                Ctor::Range(a, b) => Self::split(&head_ranges, &[(*a, *b)])
                    .into_iter()
                    .map(|(x, y)| Ctor::Range(x, y))
                    .collect(),
                other => vec![other.clone()],
            };
            let mut all = Vec::new();
            for c2 in ctors {
                let tys = Self::field_types(&rows, q, c);
                all.extend(self.useful_ctor(&rows, q, &c2, &tys, &ty)?);
                if all.len() >= WITNESS_LIMIT {
                    break;
                }
            }
            return Ok(all);
        }
        let heads: Vec<Ctor> = rows
            .iter()
            .filter_map(|r| match &r[0] {
                Pat::Ctor(c, _, _) => Some(c.clone()),
                _ => None,
            })
            .collect();
        let (present, missing): (Vec<Ctor>, Vec<Ctor>) = match self.universe(&ty) {
            Universe::Variants(tags) => tags
                .into_iter()
                .map(Ctor::Variant)
                .partition(|c| heads.contains(c)),
            Universe::Single => {
                if heads.is_empty() {
                    (Vec::new(), vec![Ctor::Single])
                } else {
                    (vec![Ctor::Single], Vec::new())
                }
            }
            Universe::Ranges(domain) => Self::split(&head_ranges, &domain)
                .into_iter()
                .map(|(x, y)| Ctor::Range(x, y))
                .partition(|c| {
                    let Ctor::Range(x, y) = c else { return false };
                    head_ranges.iter().any(|(a, b)| a <= x && y <= b)
                }),
            Universe::Infinite => (Vec::new(), vec![Ctor::Single]),
        };
        if missing.is_empty() {
            let mut all = Vec::new();
            for c in present {
                let tys = Self::field_types(&rows, q, &c);
                all.extend(self.useful_ctor(&rows, q, &c, &tys, &ty)?);
                if all.len() >= WITNESS_LIMIT {
                    break;
                }
            }
            return Ok(all);
        }
        let default: Vec<Vec<Pat>> = rows
            .iter()
            .filter(|r| matches!(r[0], Pat::Wild(_)))
            .map(|r| r[1..].to_vec())
            .collect();
        let rest = self.useful(&default, &q[1..])?;
        if rest.is_empty() {
            return Ok(rest);
        }
        let heads_wit: Vec<Wit> =
            if heads.is_empty() || matches!(self.universe(&ty), Universe::Infinite) {
                vec![Wit::Wild]
            } else {
                merge_ranges(missing)
                    .into_iter()
                    .map(|c| {
                        let arity = match &c {
                            Ctor::Variant(tag) => self.variant_arity(&ty, *tag),
                            _ => 0,
                        };
                        Wit::Ctor(c, vec![Wit::Wild; arity], ty.clone())
                    })
                    .collect()
            };
        let mut all = Vec::new();
        for h in heads_wit {
            for r in &rest {
                let mut w = vec![h.clone()];
                w.extend(r.iter().cloned());
                all.push(w);
                if all.len() >= WITNESS_LIMIT {
                    return Ok(all);
                }
            }
        }
        Ok(all)
    }

    fn useful_ctor(
        &mut self,
        rows: &[Vec<Pat>],
        q: &[Pat],
        c: &Ctor,
        tys: &[InferType],
        ty: &InferType,
    ) -> Result<Vec<Vec<Wit>>, TooComplex> {
        let spec_rows: Vec<Vec<Pat>> = rows
            .iter()
            .filter_map(|r| Self::specialize(r, c, tys))
            .collect();
        let Some(spec_q) = Self::specialize(q, c, tys) else {
            return Ok(Vec::new());
        };
        let arity = tys.len();
        Ok(self
            .useful(&spec_rows, &spec_q)?
            .into_iter()
            .map(|w| {
                let mut out = vec![Wit::Ctor(c.clone(), w[..arity].to_vec(), ty.clone())];
                out.extend(w[arity..].iter().cloned());
                out
            })
            .collect())
    }

    fn variant_arity(&self, ty: &InferType, tag: u32) -> usize {
        let InferType::Enum(name, _) = ty else {
            return 0;
        };
        self.table
            .get_enum(name)
            .and_then(|d| d.variants.iter().find(|v| v.tag == tag))
            .map_or(0, |v| v.data.len())
    }

    fn show(&self, w: &Wit) -> String {
        match w {
            Wit::Wild => "_".to_string(),
            Wit::Ctor(Ctor::Variant(tag), args, ty) => {
                let InferType::Enum(name, _) = ty else {
                    return "_".to_string();
                };
                let variant = self
                    .table
                    .get_enum(name)
                    .and_then(|d| d.variants.iter().find(|v| v.tag == *tag))
                    .map_or_else(|| "_".to_string(), |v| v.name.clone());
                let shown = source_type_name(name);
                let head = if shown == "Option" || shown == "Result" {
                    variant
                } else {
                    format!("{shown}::{variant}")
                };
                if args.is_empty() {
                    head
                } else {
                    let inner: Vec<String> = args.iter().map(|a| self.show(a)).collect();
                    format!("{head}({})", inner.join(", "))
                }
            }
            Wit::Ctor(Ctor::Single, args, ty) => match ty {
                InferType::Struct(name, _) => {
                    let names: Vec<String> = self
                        .table
                        .get_struct(name)
                        .map(|d| d.fields.iter().map(|f| f.name.clone()).collect())
                        .unwrap_or_default();
                    let inner: Vec<String> = args
                        .iter()
                        .zip(names.iter())
                        .map(|(a, n)| format!("{n}: {}", self.show(a)))
                        .collect();
                    format!("{} {{ {} }}", source_type_name(name), inner.join(", "))
                }
                _ => {
                    let inner: Vec<String> = args.iter().map(|a| self.show(a)).collect();
                    format!("({})", inner.join(", "))
                }
            },
            Wit::Ctor(Ctor::Range(lo, hi), _, ty) => show_range(*lo, *hi, ty),
            Wit::Ctor(Ctor::Str(s), _, _) => format!("{s:?}"),
        }
    }
}

fn merge_ranges(ctors: Vec<Ctor>) -> Vec<Ctor> {
    let mut out: Vec<Ctor> = Vec::new();
    for c in ctors {
        if let (Some(Ctor::Range(_, last_hi)), Ctor::Range(lo, hi)) = (out.last_mut(), &c)
            && *last_hi + 1 == *lo
        {
            *last_hi = *hi;
            continue;
        }
        out.push(c);
    }
    out
}

fn show_char(c: i128) -> String {
    match u32::try_from(c).ok().and_then(char::from_u32) {
        Some(ch) if (' '..='~').contains(&ch) && ch != '\'' && ch != '\\' => format!("'{ch}'"),
        _ => format!("'\\u{{{c:X}}}'"),
    }
}

fn show_int(v: i128, ty: &InferType) -> String {
    let Some((lo, hi)) = ty.int_bounds() else {
        return v.to_string();
    };
    if v == lo && lo != 0 {
        format!("{ty}::MIN")
    } else if v == hi {
        format!("{ty}::MAX")
    } else {
        v.to_string()
    }
}

fn show_range(lo: i128, hi: i128, ty: &InferType) -> String {
    match ty {
        InferType::Bool => match (lo, hi) {
            (0, 0) => "false".to_string(),
            (1, 1) => "true".to_string(),
            _ => "_".to_string(),
        },
        InferType::Char if lo == hi => show_char(lo),
        InferType::Char => format!("{}..={}", show_char(lo), show_char(hi)),
        _ if lo == hi => lo.to_string(),
        _ => format!("{}..={}", show_int(lo, ty), show_int(hi, ty)),
    }
}

fn children(p: &TypedPattern) -> Vec<&TypedPattern> {
    match &p.kind {
        TypedPatternKind::Binding { sub: Some(sub), .. } => vec![&**sub],
        TypedPatternKind::Variant { fields, .. }
        | TypedPatternKind::Tuple(fields)
        | TypedPatternKind::Or(fields) => fields.iter().collect(),
        TypedPatternKind::Struct { fields, .. } => fields.iter().map(|(_, f)| f).collect(),
        _ => Vec::new(),
    }
}

fn nested_or_paths(p: &TypedPattern) -> Vec<Vec<usize>> {
    fn walk(p: &TypedPattern, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, c) in children(p).into_iter().enumerate() {
            path.push(i);
            if matches!(c.kind, TypedPatternKind::Or(_)) {
                out.push(path.clone());
            }
            walk(c, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(p, &mut Vec::new(), &mut out);
    out
}

fn pattern_at<'p>(p: &'p TypedPattern, path: &[usize]) -> Option<&'p TypedPattern> {
    match path.split_first() {
        None => Some(p),
        Some((i, rest)) => pattern_at(children(p).get(*i)?, rest),
    }
}

fn replace_at(p: &TypedPattern, path: &[usize], with: TypedPattern) -> TypedPattern {
    let Some((i, rest)) = path.split_first() else {
        return with;
    };
    let mut out = p.clone();
    match &mut out.kind {
        TypedPatternKind::Binding { sub: Some(sub), .. } if *i == 0 => {
            **sub = replace_at(sub, rest, with);
        }
        TypedPatternKind::Variant { fields, .. }
        | TypedPatternKind::Tuple(fields)
        | TypedPatternKind::Or(fields) => {
            if let Some(f) = fields.get_mut(*i) {
                *f = replace_at(f, rest, with);
            }
        }
        TypedPatternKind::Struct { fields, .. } => {
            if let Some((_, f)) = fields.get_mut(*i) {
                *f = replace_at(f, rest, with);
            }
        }
        _ => {}
    }
    out
}

fn top_alternatives(p: &TypedPattern) -> Vec<&TypedPattern> {
    match &p.kind {
        TypedPatternKind::Or(alts) => alts.iter().flat_map(top_alternatives).collect(),
        _ => vec![p],
    }
}

fn flat_variant_tag(p: &TypedPattern) -> Option<u32> {
    match &p.kind {
        TypedPatternKind::Variant { tag, fields, .. }
            if fields.iter().all(|f| f.irrefutable_shallow()) =>
        {
            Some(*tag)
        }
        _ => None,
    }
}

impl TypeInference {
    pub(super) fn check_match_usefulness(
        &mut self,
        scrutinee: &TypedExpr,
        arms: &[TypedMatchArm],
        origin: MatchOrigin,
        span: Span,
    ) {
        let ty = scrutinee.ty.clone();
        let table = self.type_table.clone();
        let mut checker = Checker {
            table: &table,
            steps: 0,
        };
        let mut rows: Vec<Vec<Pat>> = Vec::new();
        let mut errors = Vec::new();
        for (i, arm) in arms.iter().enumerate() {
            let alts = top_alternatives(&arm.pattern);
            let mut useless_alts = Vec::new();
            let mut arm_rows: Vec<Vec<Pat>> = Vec::new();
            for alt in &alts {
                let q = vec![lower(alt)];
                let mut seen = rows.clone();
                seen.extend(arm_rows.iter().cloned());
                let Ok(found) = checker.useful(&seen, &q) else {
                    self.too_complex(span);
                    return;
                };
                if found.is_empty() {
                    useless_alts.push(alt.span);
                }
                arm_rows.push(q);
            }
            if useless_alts.len() == alts.len() {
                let detail = self.unreachable_text(arms, i, origin);
                errors.push(TypeError::pattern(
                    TypeErrorKind::PatternUnreachable { detail },
                    arm.pattern.span,
                ));
            } else {
                let mut earlier: Vec<Vec<Pat>> = rows.clone();
                for alt in &alts {
                    for path in nested_or_paths(alt) {
                        let Some(inner) = pattern_at(alt, &path) else {
                            continue;
                        };
                        let TypedPatternKind::Or(choices) = &inner.kind else {
                            continue;
                        };
                        for j in 1..choices.len() {
                            let before = TypedPattern::new(
                                TypedPatternKind::Or(choices[..j].to_vec()),
                                inner.ty.clone(),
                                inner.span,
                            );
                            let mut seen = earlier.clone();
                            seen.push(vec![lower(&replace_at(alt, &path, before))]);
                            let q = vec![lower(&replace_at(alt, &path, choices[j].clone()))];
                            let Ok(found) = checker.useful(&seen, &q) else {
                                self.too_complex(span);
                                return;
                            };
                            if found.is_empty() {
                                useless_alts.push(choices[j].span);
                            }
                        }
                    }
                    earlier.push(vec![lower(alt)]);
                }
                for at in useless_alts {
                    errors.push(TypeError::pattern(
                        TypeErrorKind::PatternUnreachable {
                            detail: "unreachable pattern alternative: every value it matches is \
                                     matched before it"
                                .to_string(),
                        },
                        at,
                    ));
                }
            }
            if arm.guard().is_none() {
                rows.extend(arm_rows);
            }
        }
        if !errors.is_empty() {
            self.errors.extend(errors);
            return;
        }
        let Ok(witnesses) = checker.useful(&rows, &[Pat::Wild(ty.clone())]) else {
            self.too_complex(span);
            return;
        };
        if witnesses.is_empty() {
            return;
        }
        let what = if origin == MatchOrigin::Catch {
            "catch"
        } else {
            "match"
        };
        let heads: Vec<&Wit> = witnesses.iter().map(|w| &w[0]).collect();
        let flat = heads.iter().all(|w| {
            matches!(w, Wit::Ctor(Ctor::Variant(_), args, _) if args.iter().all(|a| matches!(a, Wit::Wild)))
        });
        let shown: Vec<String> = heads.iter().map(|w| checker.show(w)).collect();
        let reason = if flat {
            let enum_name = match &ty {
                InferType::Enum(name, _) => source_type_name(name),
                _ => "",
            };
            let names: Vec<String> = heads
                .iter()
                .filter_map(|w| match w {
                    Wit::Ctor(Ctor::Variant(tag), _, ty) => {
                        let InferType::Enum(name, _) = ty else {
                            return None;
                        };
                        table
                            .get_enum(name)
                            .and_then(|d| d.variants.iter().find(|v| v.tag == *tag))
                            .map(|v| format!("'{enum_name}::{}'", v.name))
                    }
                    _ => None,
                })
                .collect();
            let mut text = format!(
                "non-exhaustive {what}: missing variant{} {}",
                if names.len() == 1 { "" } else { "s" },
                names.join(", ")
            );
            let with_payload: Vec<&String> = heads
                .iter()
                .zip(shown.iter())
                .filter(|(w, _)| matches!(w, Wit::Ctor(_, args, _) if !args.is_empty()))
                .map(|(_, s)| s)
                .collect();
            if !with_payload.is_empty() {
                let listed: Vec<String> = with_payload.iter().map(|s| format!("`{s}`")).collect();
                text.push_str(&format!("; {} not covered", listed.join(", ")));
            }
            text
        } else {
            let listed: Vec<String> = shown.iter().take(3).map(|s| format!("`{s}`")).collect();
            let more = if shown.len() > 3 { " and more" } else { "" };
            format!(
                "non-exhaustive {what}: pattern{} {}{more} not covered",
                if listed.len() == 1 { "" } else { "s" },
                listed.join(", ")
            )
        };
        self.errors.push(TypeError {
            kind: TypeErrorKind::Mismatch {
                expected: InferType::Dynamic,
                found: InferType::Dynamic,
            },
            span,
            reason: ConstraintReason::Other(reason),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        });
    }

    fn too_complex(&mut self, span: Span) {
        self.errors.push(TypeError::pattern(
            TypeErrorKind::PatternUnsupported {
                detail: "this match is too complex to check; split it into smaller matches"
                    .to_string(),
            },
            span,
        ));
    }

    fn unreachable_text(&self, arms: &[TypedMatchArm], i: usize, origin: MatchOrigin) -> String {
        let synthesized_else = i + 1 == arms.len() && i == 1;
        match origin {
            MatchOrigin::IfLet if synthesized_else => {
                return "irrefutable pattern in `if let`: the else branch is unreachable; use \
                        `match` or a plain `let`"
                    .to_string();
            }
            MatchOrigin::WhileLet if synthesized_else => {
                return "irrefutable pattern in `while let`: the loop never stops on the pattern; \
                        use `while true`"
                    .to_string();
            }
            _ => {}
        }
        let earlier = || arms[..i].iter().filter(|a| a.guard().is_none());
        if let Some(tag) = flat_variant_tag(&arms[i].pattern)
            && earlier().any(|a| flat_variant_tag(&a.pattern) == Some(tag))
            && let TypedPatternKind::Variant {
                enum_name, variant, ..
            } = &arms[i].pattern.kind
        {
            return format!(
                "unreachable match arm: duplicate pattern for variant '{}::{}'",
                source_type_name(enum_name),
                variant
            );
        }
        if earlier().any(|a| matches!(a.pattern.kind, TypedPatternKind::Wildcard)) {
            return "unreachable match arm: wildcard pattern must be the last match arm"
                .to_string();
        }
        "unreachable match arm: every value it matches is matched by an earlier arm".to_string()
    }
}
