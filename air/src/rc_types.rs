use crate::layout::{enum_payload_offset, payload_field_offsets};
use crate::rc_paths::{RcLeafPath, RcPathStep, RcScan, rc_field_paths};
use crate::{AirProgram, AirStmtKind, AirType};

#[derive(Debug, Clone)]
pub struct RcTypeEntry {
    pub ty: AirType,
    pub type_id: u32,
    pub pointer_offsets: Vec<u32>,
    pub pointer_guards: Vec<Option<(u32, u32)>>,
}
#[derive(Debug, Clone, Default)]
pub struct RcTypeTable {
    pub entries: Vec<RcTypeEntry>,
}

impl RcTypeTable {
    pub fn lookup_id(&self, ty: &AirType) -> Option<u32> {
        self.entries.iter().find(|e| &e.ty == ty).map(|e| e.type_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcOffsetError(pub String);

impl std::fmt::Display for RcOffsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rc pointer-map: {}", self.0)
    }
}

impl std::error::Error for RcOffsetError {}

pub fn compute_offset_for_path(
    data_ty: &AirType,
    path: &RcLeafPath,
    program: &AirProgram,
) -> Result<u32, RcOffsetError> {
    let (acc, cur) = offset_of_steps(data_ty, &path.steps, program)?;
    if !matches!(cur, AirType::Rc(_)) {
        return Err(RcOffsetError(format!(
            "Rc path does not terminate on an rc leaf (ended on {cur:?})"
        )));
    }
    Ok(acc)
}

fn offset_of_steps(
    data_ty: &AirType,
    steps: &[RcPathStep],
    program: &AirProgram,
) -> Result<(u32, AirType), RcOffsetError> {
    let mut acc: u32 = 0;
    let mut cur: AirType = data_ty.clone();

    for step in steps {
        match step {
            RcPathStep::Field(field_name) => {
                let AirType::Struct(struct_ref) = &cur else {
                    return Err(RcOffsetError(format!(
                        "Field step `{field_name}` on non-struct type {cur:?}"
                    )));
                };
                let struct_name = struct_ref.symbol();
                let def = program
                    .structs
                    .iter()
                    .find(|s| s.name == struct_name)
                    .ok_or_else(|| {
                        RcOffsetError(format!("unknown struct `{struct_name}` along Rc path"))
                    })?;
                let field = def
                    .fields
                    .iter()
                    .find(|f| &f.name == field_name)
                    .ok_or_else(|| {
                        RcOffsetError(format!(
                            "unknown field `{field_name}` on struct `{struct_name}`"
                        ))
                    })?;
                let off = field.offset.ok_or_else(|| {
                    RcOffsetError(format!(
                        "field `{field_name}` on struct `{struct_name}` has no computed offset \
                         (collect_rc_types must run after compute_layouts)"
                    ))
                })?;
                acc += off;
                cur = field.ty.clone();
            }
            RcPathStep::EnumPayload {
                enum_ref,
                tag,
                field_index,
            } => {
                let enum_name = enum_ref.symbol();
                let def = program
                    .enums
                    .iter()
                    .find(|e| e.name == enum_name)
                    .ok_or_else(|| {
                        RcOffsetError(format!("unknown enum `{enum_name}` along Rc path"))
                    })?;
                let variant = def.variants.iter().find(|v| v.tag == *tag).ok_or_else(|| {
                    RcOffsetError(format!("unknown variant tag {tag} on enum `{enum_name}`"))
                })?;
                let idx = *field_index as usize;
                if idx >= variant.payload.len() {
                    return Err(RcOffsetError(format!(
                        "payload field index {field_index} out of range on enum `{enum_name}` \
                         variant tag {tag} (len {})",
                        variant.payload.len()
                    )));
                }

                let intra = payload_field_offsets(&variant.payload, &program.struct_sizes)[idx];
                acc += enum_payload_offset(def, &program.struct_sizes) + intra;
                cur = variant.payload[idx].clone();
            }
        }
    }
    Ok((acc, cur))
}

// id 0 is reserved for the inert sentinel (__aelys_vec_new), so real ids start at 1
pub fn collect_rc_types(program: &AirProgram) -> Result<RcTypeTable, RcOffsetError> {
    let mut entries: Vec<RcTypeEntry> = Vec::new();

    for func in &program.functions {
        for block in &func.blocks {
            for stmt in &block.stmts {
                let AirStmtKind::RcAlloc { ty, .. } = &stmt.kind else {
                    continue;
                };
                if entries.iter().any(|e| &e.ty == ty) {
                    continue;
                }
                let type_id = entries.len() as u32 + 1;
                let (pointer_offsets, pointer_guards) = pointer_offsets_for(ty, program)?;
                entries.push(RcTypeEntry {
                    ty: ty.clone(),
                    type_id,
                    pointer_offsets,
                    pointer_guards,
                });
            }
        }
    }

    Ok(RcTypeTable { entries })
}

type PointerMap = (Vec<u32>, Vec<Option<(u32, u32)>>);

fn pointer_offsets_for(ty: &AirType, program: &AirProgram) -> Result<PointerMap, RcOffsetError> {
    if matches!(
        rc_field_paths(ty, &program.structs, &program.enums),
        RcScan::Undecidable(_) | RcScan::RejectedMultiVariant(_)
    ) {
        return Ok((Vec::new(), Vec::new()));
    }
    let leaves = crate::rc_paths::rc_traced_leaves(
        ty,
        crate::rc_paths::Defs::local(&program.structs, &program.enums),
    )
    .map_err(RcOffsetError)?;
    let mut offsets = Vec::with_capacity(leaves.len());
    let mut guards = Vec::with_capacity(leaves.len());
    for leaf in &leaves {
        offsets.push(compute_offset_for_path(ty, &leaf.path, program)?);
        guards.push(match leaf.guard {
            None => None,
            Some(at) => {
                let (tag_off, _) = offset_of_steps(ty, &leaf.path.steps[..at], program)?;
                let RcPathStep::EnumPayload { tag, .. } = &leaf.path.steps[at] else {
                    return Err(RcOffsetError("a guard names a step that is not a payload".into()));
                };
                Some((tag_off, *tag))
            }
        });
    }
    Ok((offsets, guards))
}
