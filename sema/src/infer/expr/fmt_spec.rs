use super::TypeInference;
use crate::constraint::TypeError;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, Span};

const MAX_WIDTH: i64 = 4096;
const MAX_PRECISION: i64 = 64;
const FORM: &str = "`[[fill]align][0][width][.precision]`, align one of `<`, `>`, `^`, a count written `n`, `n$` or `name$`";

enum Count {
    Fixed(i64),
    Arg,
}

struct FmtSpec {
    fill: char,
    align: Option<char>,
    zero: bool,
    width: Option<Count>,
    precision: Option<Count>,
}

fn count(chars: &[char], at: &mut usize) -> Option<Count> {
    let start = *at;
    while chars
        .get(*at)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        *at += 1;
    }
    if *at > start && chars.get(*at) == Some(&'$') {
        *at += 1;
        return Some(Count::Arg);
    }
    *at = start;
    digits(chars, at).map(Count::Fixed)
}

fn digits(chars: &[char], at: &mut usize) -> Option<i64> {
    let start = *at;
    let mut n: i64 = 0;
    while let Some(c) = chars.get(*at).and_then(|c| c.to_digit(10)) {
        n = n.saturating_mul(10).saturating_add(c as i64);
        *at += 1;
    }
    (*at > start).then_some(n)
}

fn parse_spec(spec: &str) -> Option<FmtSpec> {
    let chars: Vec<char> = spec.chars().collect();
    let is_align = |c: Option<&char>| matches!(c, Some('<' | '>' | '^'));
    let mut at = 0;
    let mut out = FmtSpec {
        fill: ' ',
        align: None,
        zero: false,
        width: None,
        precision: None,
    };
    if chars.len() >= 2 && is_align(chars.get(1)) {
        out.fill = chars[0];
        out.align = Some(chars[1]);
        at = 2;
    } else if is_align(chars.first()) {
        out.align = Some(chars[0]);
        at = 1;
    }
    if chars.get(at) == Some(&'0') && chars.get(at + 1).is_some_and(|c| c.is_ascii_digit()) {
        out.zero = true;
        at += 1;
    }
    out.width = count(&chars, &mut at);
    if chars.get(at) == Some(&'.') {
        at += 1;
        out.precision = Some(count(&chars, &mut at)?);
    }
    if at != chars.len() || (out.width.is_none() && out.precision.is_none()) {
        return None;
    }
    Some(out)
}

impl TypeInference {
    fn fmt_spec_error(&mut self, message: String, span: Span) {
        self.errors.push(TypeError::coded(
            "E0391",
            message,
            "format specification",
            span,
        ));
    }

    pub(super) fn infer_fmt_spec(
        &mut self,
        value: &Expr,
        spec: &Expr,
        counts: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed = self.infer_expr(value);
        let ExprKind::String(text) = &spec.kind else {
            return (TypedExprKind::Null, InferType::Dynamic);
        };
        let ty = match &typed.ty {
            InferType::Var(_) => self.probe_type(&typed.ty),
            other => other.clone(),
        };
        let Some(parsed) = parse_spec(text) else {
            self.fmt_spec_error(
                format!("unknown format specification `:{text}`; the form is {FORM}"),
                span,
            );
            return (TypedExprKind::Null, InferType::Dynamic);
        };
        if matches!(ty, InferType::Var(_) | InferType::Dynamic) {
            self.fmt_spec_error(
                format!(
                    "the type of a value formatted with `:{text}` is not known here; annotate \
                     the binding"
                ),
                span,
            );
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        if matches!(parsed.width, Some(Count::Fixed(w)) if w > MAX_WIDTH) {
            self.fmt_spec_error(format!("the width in `:{text}` is above {MAX_WIDTH}"), span);
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        if matches!(parsed.precision, Some(Count::Fixed(p)) if p > MAX_PRECISION) {
            self.fmt_spec_error(
                format!("the precision in `:{text}` is above {MAX_PRECISION}"),
                span,
            );
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        if parsed.zero && !ty.is_numeric() {
            self.fmt_spec_error(
                format!(
                    "`:{text}` pads with zeros, which applies to a number, and this value is \
                     `{ty}`; pad it with an alignment, as `:>{}`",
                    text.trim_start_matches('0')
                ),
                span,
            );
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        if parsed.zero && parsed.align.is_some() {
            self.fmt_spec_error(
                format!("`:{text}` gives both `0` and an alignment; the form is {FORM}"),
                span,
            );
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        if parsed.precision.is_some() && !ty.is_float() {
            self.fmt_spec_error(
                format!(
                    "the precision in `:{text}` applies to `f32` and `f64`, and this value is \
                     `{ty}`"
                ),
                span,
            );
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        let int = |n: i64| TypedExpr::new(TypedExprKind::Int(n), InferType::I64, span);
        let mut counts = counts.iter();
        let mut count_value = |this: &mut Self, c: Count| match c {
            Count::Fixed(n) => Some(int(n)),
            Count::Arg => {
                let arg = this.infer_expr(counts.next()?);
                if arg.ty == InferType::I64 {
                    return Some(arg);
                }
                if !arg.ty.is_integer() {
                    this.fmt_spec_error(
                        format!(
                            "a count in `:{text}` is an integer, and this argument is `{}`",
                            arg.ty
                        ),
                        span,
                    );
                    return None;
                }
                Some(TypedExpr::new(
                    TypedExprKind::Cast {
                        expr: Box::new(arg),
                        target: InferType::I64,
                    },
                    InferType::I64,
                    span,
                ))
            }
        };
        let width = match parsed.width {
            Some(c) => match count_value(self, c) {
                Some(w) => Some(w),
                None => return (TypedExprKind::Null, InferType::Dynamic),
            },
            None => None,
        };
        let precision = match parsed.precision {
            Some(c) => match count_value(self, c) {
                Some(p) => Some(p),
                None => return (TypedExprKind::Null, InferType::Dynamic),
            },
            None => None,
        };
        let rendered = match precision {
            Some(p) => {
                let as_f64 = if ty == InferType::F64 {
                    typed
                } else {
                    TypedExpr::new(
                        TypedExprKind::Cast {
                            expr: Box::new(typed),
                            target: InferType::F64,
                        },
                        InferType::F64,
                        span,
                    )
                };
                TypedExpr::new(
                    TypedExprKind::EnumVariant {
                        enum_name: "string".to_string(),
                        variant: "fmt_precision".to_string(),
                        tag: 0,
                        args: vec![as_f64, p],
                    },
                    InferType::String,
                    span,
                )
            }
            None => TypedExpr::new(
                TypedExprKind::FmtString(vec![TypedFmtStringPart::Expr(Box::new(typed))]),
                InferType::String,
                span,
            ),
        };
        let Some(width) = width else {
            return (rendered.kind, InferType::String);
        };
        let align = match parsed.align {
            Some('<') => 0,
            Some('^') => 2,
            Some(_) => 1,
            None if ty.is_numeric() => 1,
            None => 0,
        };
        let fill = if parsed.zero { '0' } else { parsed.fill };
        (
            TypedExprKind::EnumVariant {
                enum_name: "string".to_string(),
                variant: "fmt_pad".to_string(),
                tag: 0,
                args: vec![
                    rendered,
                    width,
                    int(align),
                    int(fill as i64),
                    int(i64::from(parsed.zero)),
                ],
            },
            InferType::String,
        )
    }
}
