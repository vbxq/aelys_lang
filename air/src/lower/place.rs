use super::LoweringContext;
use crate::*;
use aelys_sema::{TypedExpr, TypedExprKind, deref_is_shared};

/// where a place chain bottoms out. `pointee` means the chain was entered through a pointer
/// value, so the storage it names is not a local of this function.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum PlaceRoot {
    Local(LocalId),
    Global(String),
    Pointee,
}

pub(super) struct Addr {
    /// an air local of type `ptr(pointee)`: the address of the place, never of a copy
    pub ptr: LocalId,
    pub pointee: AirType,
    pub root: PlaceRoot,
    /// the chain dereferenced a shared `&`; sema owns the decision, this only reports it
    #[allow(dead_code)]
    pub shared: bool,
}

impl<'a> LoweringContext<'a> {
    /// the canonical place-address computation for user places. `none` iff the expression
    pub(super) fn place_addr(&mut self, e: &TypedExpr) -> Option<Addr> {
        let sp = Some(self.span(&e.span));
        match &e.kind {
            TypedExprKind::Grouping(inner) => self.place_addr(inner),

            TypedExprKind::Identifier(name) => {
                if let Some(id) = self.lookup_local(name) {
                    if self.capture_slots.contains_key(&id) {
                        let pointee = self.lower_type_from_infer(&e.ty);
                        return Some(self.capture_addr(id, pointee));
                    }
                    let pointee = self
                        .local_air_type(id)
                        .unwrap_or_else(|| self.lower_type_from_infer(&e.ty));
                    let ptr = self.emit_addr_of(Place::Local(id), &pointee, sp);
                    Some(Addr {
                        ptr,
                        pointee,
                        root: PlaceRoot::Local(id),
                        shared: false,
                    })
                } else if self.globals.iter().any(|g| g.name == *name) {
                    let pointee = self.lower_type_from_infer(&e.ty);
                    let ptr = self.emit_addr_of(Place::Global(name.clone()), &pointee, sp);
                    Some(Addr {
                        ptr,
                        pointee,
                        root: PlaceRoot::Global(name.clone()),
                        shared: false,
                    })
                } else {
                    None
                }
            }

            // the loaded pointee would address a stack temp
            TypedExprKind::Deref(inner) => {
                let op = self.lower_expr(inner);
                let ptr_ty = self.lower_type_from_infer(&inner.ty);
                let ptr = self.operand_to_local(op, &ptr_ty);
                Some(Addr {
                    ptr,
                    pointee: self.lower_type_from_infer(&e.ty),
                    root: PlaceRoot::Pointee,
                    shared: deref_is_shared(&inner.ty),
                })
            }

            // shared borrow, so `shared` stays false and `rc::get(rc).f = v` becomes correct
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                args,
                ..
            } if enum_name == "Rc" && variant == "get" => {
                let handle = args.first()?;
                let op = self.lower_expr(handle);
                let ptr_ty = self.lower_type_from_infer(&handle.ty);
                let ptr = self.operand_to_local(op, &ptr_ty);
                Some(Addr {
                    ptr,
                    pointee: self.lower_type_from_infer(&e.ty),
                    root: PlaceRoot::Pointee,
                    shared: false,
                })
            }

            TypedExprKind::Member { object, member } => {
                let base = self.projection_base(object)?;
                let pointee = self.lower_type_from_infer(&e.ty);
                let ptr = self.emit_addr_of(Place::Field(base.ptr, member.clone()), &pointee, sp);
                Some(Addr {
                    ptr,
                    pointee,
                    root: base.root,
                    shared: base.shared,
                })
            }

            TypedExprKind::Index { object, index } => {
                let idx = self.lower_expr(index);
                let base = self.projection_base(object)?;
                let pointee = self.lower_type_from_infer(&e.ty);
                let ptr = self.emit_addr_of(Place::Index(base.ptr, idx), &pointee, sp);
                Some(Addr {
                    ptr,
                    pointee,
                    root: base.root,
                    shared: base.shared,
                })
            }

            _ => None,
        }
    }

    /// pointer an `rc` handle, a reference, an `rc`-typed field the projection auto-derefs
    pub(super) fn projection_base(&mut self, object: &TypedExpr) -> Option<Addr> {
        if let AirType::Ptr(inner) = self.lower_type_from_infer(&object.ty) {
            let ptr_ty = AirType::Ptr(inner.clone());
            let op = self.lower_expr(object);
            let ptr = self.operand_to_local(op, &ptr_ty);
            return Some(Addr {
                ptr,
                pointee: *inner,
                root: PlaceRoot::Pointee,
                shared: deref_is_shared(&object.ty),
            });
        }
        self.place_addr(object)
    }

    /// only reason it is not `place_addr(<the capture identifier>)` is that the binding it
    pub(super) fn addr_of_env_field(
        &mut self,
        env: LocalId,
        field: &str,
        pointee: &AirType,
    ) -> LocalId {
        self.emit_addr_of(Place::Field(env, field.to_string()), pointee, None)
    }

    /// a capture reads and writes through the env-field pointer the prologue computed, so its
    /// address is that pointer, with no second addressof.
    pub(super) fn capture_addr(&mut self, cap_ptr: LocalId, pointee: AirType) -> Addr {
        Addr {
            ptr: cap_ptr,
            pointee,
            root: PlaceRoot::Pointee,
            shared: false,
        }
    }

    /// the only other entry point: the address of a temp this lowering just allocated. its
    pub(super) fn addr_of_own_temp(
        &mut self,
        t: LocalId,
        ty: &AirType,
        sp: Option<Span>,
    ) -> Operand {
        Operand::Copy(self.emit_addr_of(Place::Local(t), ty, sp))
    }

    fn emit_addr_of(&mut self, place: Place, pointee: &AirType, sp: Option<Span>) -> LocalId {
        let tmp = self.alloc_temp(AirType::Ptr(Box::new(pointee.clone())));
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(tmp),
                rvalue: Rvalue::AddressOf(place),
            },
            sp,
        );
        tmp
    }
}
