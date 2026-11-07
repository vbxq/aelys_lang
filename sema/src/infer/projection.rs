use super::TypeInference;
use crate::constraint::TypeError;
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use crate::unify::{Dir, Substitution, unify_dir};
use aelys_syntax::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjKind {
    Field(String),
    Index,
    Slice,
    Element,
    Deref,
    FieldWrite(String),
    IndexWrite,
    DerefWrite,
}

impl ProjKind {
    fn tag(&self) -> u8 {
        match self {
            ProjKind::Field(_) => 0,
            ProjKind::Index => 1,
            ProjKind::Slice => 2,
            ProjKind::Element => 3,
            ProjKind::Deref => 4,
            ProjKind::FieldWrite(_) => 5,
            ProjKind::IndexWrite => 6,
            ProjKind::DerefWrite => 7,
        }
    }

    fn is_write(&self) -> bool {
        matches!(
            self,
            ProjKind::FieldWrite(_) | ProjKind::IndexWrite | ProjKind::DerefWrite
        )
    }

    fn dir(&self) -> Dir {
        match self {
            ProjKind::FieldWrite(_) | ProjKind::IndexWrite => Dir::Flow,
            ProjKind::DerefWrite => Dir::FlowRev,
            _ => Dir::Exact,
        }
    }

    fn what(&self) -> String {
        match self {
            ProjKind::Field(m) => format!("field `{m}`"),
            ProjKind::Index => "element".to_string(),
            ProjKind::Slice => "slice".to_string(),
            ProjKind::Element => "loop element".to_string(),
            ProjKind::Deref => "referent".to_string(),
            ProjKind::FieldWrite(m) => format!("assignment to field `{m}`"),
            ProjKind::IndexWrite => "element assignment".to_string(),
            ProjKind::DerefWrite => "assignment through a reference".to_string(),
        }
    }

    fn unresolved(&self) -> String {
        match self {
            ProjKind::Field(m) => format!("its field `{m}` cannot be resolved"),
            ProjKind::Index => "it cannot be indexed".to_string(),
            ProjKind::Slice => "it cannot be sliced".to_string(),
            ProjKind::Element => "it cannot be iterated".to_string(),
            ProjKind::Deref => "it cannot be dereferenced".to_string(),
            ProjKind::FieldWrite(m) => format!("its field `{m}` cannot be assigned"),
            ProjKind::IndexWrite => "its element cannot be assigned".to_string(),
            ProjKind::DerefWrite => "it cannot be assigned through".to_string(),
        }
    }
}

pub(crate) type SiteKey = (usize, usize, u8);

fn site_key(span: Span, kind: &ProjKind) -> SiteKey {
    (span.start, span.end, kind.tag())
}

#[derive(Debug, Clone)]
pub(crate) enum CarrierCheck {
    Surface {
        ty: InferType,
        generic: bool,
        span: Span,
        what: String,
    },
    AggregateElement {
        ty: InferType,
        kind: String,
        span: Span,
    },
    VecPush {
        ty: InferType,
        span: Span,
    },
}

impl CarrierCheck {
    fn ty(&self) -> &InferType {
        match self {
            CarrierCheck::Surface { ty, .. }
            | CarrierCheck::AggregateElement { ty, .. }
            | CarrierCheck::VecPush { ty, .. } => ty,
        }
    }

    fn resolved(&self, subst: &Substitution) -> CarrierCheck {
        let mut out = self.clone();
        match &mut out {
            CarrierCheck::Surface { ty, .. }
            | CarrierCheck::AggregateElement { ty, .. }
            | CarrierCheck::VecPush { ty, .. } => *ty = subst.apply(ty),
        }
        out
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DeferredProjection {
    object: InferType,
    kind: ProjKind,
    target: InferType,
    span: Span,
}

impl TypeInference {
    pub(crate) fn defer_read(
        &mut self,
        object: &InferType,
        kind: ProjKind,
        span: Span,
    ) -> InferType {
        let result = self.type_gen.fresh();
        if let InferType::Var(id) = result {
            self.deferred_results.insert(id);
        }
        self.record_deferred(object, kind, result.clone(), span);
        result
    }

    pub(crate) fn defer_write(
        &mut self,
        object: &InferType,
        kind: ProjKind,
        value: &InferType,
        span: Span,
    ) {
        self.record_deferred(object, kind, value.clone(), span);
    }

    fn record_deferred(
        &mut self,
        object: &InferType,
        kind: ProjKind,
        target: InferType,
        span: Span,
    ) {
        self.deferred_sites.insert(site_key(span, &kind));
        self.deferred.push(DeferredProjection {
            object: object.clone(),
            kind,
            target,
            span,
        });
    }

    pub(crate) fn check_carrier(&mut self, check: CarrierCheck) {
        let before = self.errors.len();
        self.run_carrier_check(&check);
        if self.errors.len() == before && check.ty().has_vars() {
            self.deferred_carriers.push(check);
        }
    }

    fn run_carrier_check(&mut self, check: &CarrierCheck) {
        match check {
            CarrierCheck::Surface {
                ty,
                generic,
                span,
                what,
            } => self.reject_rc_out_of_carrier_surface(ty, *generic, *span, what),
            CarrierCheck::AggregateElement { ty, kind, span } => {
                self.reject_rc_aggregate_element(ty, kind, *span)
            }
            CarrierCheck::VecPush { ty, span } => self.reject_rc_vec_push(ty, *span),
        }
    }

    pub(crate) fn replay_carrier_checks(&mut self, subst: &Substitution) {
        for check in std::mem::take(&mut self.deferred_carriers) {
            self.run_carrier_check(&check.resolved(subst));
        }
    }

    pub(crate) fn names_a_deferred_result(&self, ty: &InferType) -> bool {
        matches!(ty, InferType::Var(id) if self.deferred_results.contains(id))
    }

    fn struct_field_type(&self, object: &InferType, field: &str) -> Option<InferType> {
        let name = match object {
            InferType::Struct(name) => name,
            InferType::Rc(inner) => match inner.as_ref() {
                InferType::Struct(name) => name,
                _ => return None,
            },
            _ => return None,
        };
        self.type_table
            .get_struct(name)?
            .fields
            .iter()
            .find(|f| f.name == field)
            .map(|f| f.ty.clone())
    }

    pub(crate) fn project(&self, object: &InferType, kind: &ProjKind) -> Option<InferType> {
        let elem = match object {
            InferType::Array(inner, _) | InferType::Vec(inner) => Some((**inner).clone()),
            InferType::Slice { elem, .. } => Some((**elem).clone()),
            _ => None,
        };
        match kind {
            ProjKind::Field(member) => match object {
                InferType::String => match member.as_str() {
                    "len" => Some(InferType::I64),
                    "bytes" => Some(InferType::Slice {
                        elem: Box::new(InferType::U8),
                        mutable: false,
                    }),
                    _ => None,
                },
                InferType::Slice { .. } | InferType::Vec(_) | InferType::Array(..) => {
                    (member == "len").then_some(InferType::I64)
                }
                _ => self.struct_field_type(object, member),
            },
            ProjKind::FieldWrite(field) => self.struct_field_type(object, field),
            ProjKind::Index | ProjKind::Slice | ProjKind::IndexWrite => elem,
            ProjKind::Element => match object {
                InferType::String => Some(InferType::Char),
                _ => elem,
            },
            ProjKind::Deref | ProjKind::DerefWrite => match object {
                InferType::Ref { referent, .. } => Some((**referent).clone()),
                _ => None,
            },
        }
    }

    fn agree(left: &InferType, right: &InferType, dir: Dir) -> bool {
        unify_dir(left, right, &mut Substitution::new(), dir).is_ok()
    }

    fn mismatch_for(
        &self,
        kind: &ProjKind,
        target: InferType,
        projected: InferType,
        span: Span,
    ) -> TypeError {
        if kind.is_write() {
            TypeError::late_projection(projected, target, &kind.what(), span)
        } else {
            TypeError::late_projection(target, projected, &kind.what(), span)
        }
    }

    pub(super) fn resolve_deferred(&mut self, pending: &mut Vec<usize>, subst: &mut Substitution) {
        loop {
            let mut progress = false;
            let mut still = Vec::with_capacity(pending.len());
            for &i in pending.iter() {
                let object = subst.apply(&self.deferred[i].object);
                if matches!(object, InferType::Var(_)) {
                    still.push(i);
                    continue;
                }
                let d = self.deferred[i].clone();
                progress = true;
                let projected = if matches!(object, InferType::Dynamic) {
                    Some(InferType::Dynamic)
                } else {
                    self.project(&object, &d.kind)
                };
                let Some(projected) = projected else {
                    if !d.kind.is_write() {
                        let _ = unify_dir(&d.target, &InferType::Dynamic, subst, Dir::Exact);
                    }
                    continue;
                };
                let (left, right) = if matches!(d.kind, ProjKind::DerefWrite) {
                    (projected.clone(), d.target.clone())
                } else {
                    (d.target.clone(), projected.clone())
                };
                let saved = subst.snapshot();
                if unify_dir(&left, &right, subst, d.kind.dir()).is_err() {
                    subst.restore(saved);
                    let target = subst.apply(&d.target);
                    let err = self.mismatch_for(&d.kind, target, projected, d.span);
                    self.errors.push(err);
                    self.reported_sites.insert(site_key(d.span, &d.kind));
                }
            }
            *pending = still;
            if !progress || pending.is_empty() {
                break;
            }
        }
    }

    pub(super) fn report_unresolved(&mut self, pending: &[usize]) {
        for &i in pending {
            let d = self.deferred[i].clone();
            self.errors.push(TypeError::unresolved_projection(
                &d.kind.unresolved(),
                d.span,
            ));
            self.reported_sites.insert(site_key(d.span, &d.kind));
        }
    }

    pub(super) fn is_deferred_site(&self, span: Span, kind: &ProjKind) -> bool {
        self.deferred_sites.contains(&site_key(span, kind))
    }

    fn deferred_site_open(&self, span: Span, kind: &ProjKind) -> bool {
        let key = site_key(span, kind);
        self.deferred_sites.contains(&key) && !self.reported_sites.contains(&key)
    }

    pub(super) fn check_deferred_read(
        &mut self,
        kind: ProjKind,
        object: &TypedExpr,
        index: Option<&TypedExpr>,
        seen: &InferType,
        span: Span,
    ) {
        if !self.deferred_site_open(span, &kind) {
            return;
        }
        if matches!(object.ty, InferType::Var(_) | InferType::Dynamic) {
            return;
        }
        if let (ProjKind::Index, Some(index)) = (&kind, index) {
            self.reject_const_index_out_of_bounds(object, index);
        }
        match self.project(&object.ty, &kind) {
            Some(projected) => {
                let unlinked =
                    matches!(seen, InferType::Dynamic) && !matches!(projected, InferType::Dynamic);
                if unlinked || !Self::agree(seen, &projected, Dir::Exact) {
                    let err = self.mismatch_for(&kind, seen.clone(), projected, span);
                    self.errors.push(err);
                }
            }
            None => match (&kind, &object.ty) {
                (ProjKind::Index, InferType::String) => {
                    self.errors
                        .push(TypeError::string_index_removed(false, object.span));
                }
                (ProjKind::Deref, other) => {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "cannot dereference a non-reference value of type `{}`",
                            other
                        ),
                        span,
                    ));
                }
                _ => {}
            },
        }
    }

    pub(super) fn check_deferred_write(
        &mut self,
        kind: ProjKind,
        object: &TypedExpr,
        index: Option<&TypedExpr>,
        value: &TypedExpr,
        span: Span,
    ) {
        if !self.deferred_site_open(span, &kind) {
            return;
        }
        if matches!(object.ty, InferType::Var(_) | InferType::Dynamic) {
            return;
        }
        let owner = match &object.ty {
            InferType::Struct(name) => Some(name.clone()),
            InferType::Rc(inner) => match inner.as_ref() {
                InferType::Struct(name) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        };
        if let (ProjKind::FieldWrite(field), Some(owner)) = (&kind, &owner) {
            self.reject_private_field(owner, field, span);
        }
        let Some(projected) = self.project(&object.ty, &kind) else {
            match &kind {
                ProjKind::IndexWrite if matches!(object.ty, InferType::String) => {
                    self.errors
                        .push(TypeError::string_index_removed(true, object.span));
                }
                ProjKind::FieldWrite(field) => match &owner {
                    Some(owner) if self.type_table.has_struct(owner) => {
                        self.errors.push(TypeError::member_access(
                            format!("struct '{}' has no field '{}'", owner, field),
                            span,
                        ));
                    }
                    Some(_) => {}
                    None if !crate::place_spine::is_computed_len(&object.ty, field) => {
                        self.errors.push(TypeError::member_access(
                            format!("field assignment on non-struct type {}", object.ty),
                            span,
                        ));
                    }
                    None => {}
                },
                ProjKind::DerefWrite => {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "cannot assign through a non-reference value of type `{}`",
                            object.ty
                        ),
                        span,
                    ));
                }
                _ => {}
            }
            return;
        };
        if let (ProjKind::IndexWrite, Some(index)) = (&kind, index) {
            self.reject_const_index_out_of_bounds(object, index);
        }
        let agrees = if matches!(kind, ProjKind::DerefWrite) {
            Self::agree(&projected, &value.ty, kind.dir())
        } else {
            Self::agree(&value.ty, &projected, kind.dir())
        };
        if !agrees {
            let err = self.mismatch_for(&kind, value.ty.clone(), projected.clone(), span);
            self.errors.push(err);
        }
        match &kind {
            ProjKind::FieldWrite(field) => {
                self.reject_rc_field_write(field, &projected, object, value, span);
            }
            ProjKind::IndexWrite => self.reject_vec_into_element(&object.ty, value),
            _ => {}
        }
    }

    pub(super) fn is_direct_rc_producer(kind: &TypedExprKind) -> bool {
        matches!(
            kind,
            TypedExprKind::EnumVariant { .. }
                | TypedExprKind::Identifier(_)
                | TypedExprKind::Member { .. }
                | TypedExprKind::Call { .. }
        )
    }
}
