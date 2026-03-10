use crate::{AirEnumDef, AirProgram, AirStructDef, AirType};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy)]
pub struct TypeLayout {
    pub size: u32,
    pub align: u32,
}

pub fn layout_of(ty: &AirType) -> TypeLayout {
    match ty {
        AirType::I8 | AirType::U8 | AirType::Bool => TypeLayout { size: 1, align: 1 },
        AirType::I16 | AirType::U16 => TypeLayout { size: 2, align: 2 },
        AirType::I32 | AirType::U32 | AirType::F32 => TypeLayout { size: 4, align: 4 },
        AirType::I64 | AirType::U64 | AirType::F64 => TypeLayout { size: 8, align: 8 },
        AirType::Ptr(_) | AirType::FnPtr { .. } => TypeLayout { size: 8, align: 8 },
        AirType::Str => TypeLayout { size: 16, align: 8 },
        AirType::Void => TypeLayout { size: 0, align: 1 },
        AirType::Slice(_) => TypeLayout { size: 16, align: 8 },
        AirType::Param(_) | AirType::Opaque => TypeLayout { size: 8, align: 8 },
        AirType::Array(inner, n) => {
            let el = layout_of(inner);
            TypeLayout {
                size: el.size * (*n as u32),
                align: el.align,
            }
        }
        // Simple enum layout (tag only). Data enums use their registered LLVM struct
        // type in codegen, so this is only used for simple enums without data variants.
        AirType::Enum(_) => TypeLayout { size: 4, align: 4 },
        AirType::Struct(name) => {
            panic!("layout_of: Struct({name}) requires program context; run compute_layouts first")
        }
    }
}

pub fn compute_layouts(program: &mut AirProgram) {
    let name_to_idx: HashMap<String, usize> = program
        .structs
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.clone(), i))
        .collect();

    detect_self_references(&program.structs);
    let order = topological_order(&program.structs, &name_to_idx);

    let mut resolved: HashMap<String, TypeLayout> = HashMap::new();

    for idx in order {
        let (total, offsets) = struct_layout(&program.structs[idx], &resolved);
        resolved.insert(program.structs[idx].name.clone(), total);
        for (i, off) in offsets.into_iter().enumerate() {
            program.structs[idx].fields[i].offset = Some(off);
        }
    }

    // Compute enum sizes using a fixed-point loop so that nested enums
    // (e.g., Option<Option<i64>>) are resolved in dependency order.
    let mut remaining: Vec<usize> = (0..program.enums.len()).collect();
    let max_iterations = remaining.len() + 1;
    for _ in 0..max_iterations {
        if remaining.is_empty() {
            break;
        }
        let mut next_remaining = Vec::new();
        for &idx in &remaining {
            let def = &program.enums[idx];
            if !enum_has_data(def) {
                resolved.insert(def.name.clone(), TypeLayout { size: 4, align: 4 });
                continue;
            }
            // Check if all payload types can be resolved
            let all_resolved = def.variants.iter().all(|v| {
                v.payload.iter().all(|ty| match ty {
                    AirType::Enum(name) | AirType::Struct(name) => resolved.contains_key(name.as_str()),
                    _ => true,
                })
            });
            if !all_resolved {
                next_remaining.push(idx);
                continue;
            }
            let payload_size = enum_max_payload_size(def, &resolved);
            if payload_size == 0 {
                resolved.insert(def.name.clone(), TypeLayout { size: 4, align: 4 });
            } else {
                // Enum layout: { i32 tag, [payload_size x i8] }
                // Tag is 4 bytes (i32), payload follows with max alignment
                let payload_align = enum_max_payload_align(def, &resolved);
                let total_align = 4u32.max(payload_align);
                let payload_offset = align_to(4, payload_align);
                let total_size = align_to(payload_offset + payload_size, total_align);
                resolved.insert(
                    def.name.clone(),
                    TypeLayout {
                        size: total_size,
                        align: total_align,
                    },
                );
            }
        }
        remaining = next_remaining;
    }

    program.struct_sizes = resolved;
}

pub fn resolved_layout(ty: &AirType, sizes: &HashMap<String, TypeLayout>) -> TypeLayout {
    match ty {
        AirType::Struct(name) => *sizes
            .get(name.as_str())
            .unwrap_or_else(|| panic!("struct `{name}` referenced before its layout is computed")),
        AirType::Enum(name) => {
            // Look up pre-computed enum size. Falls back to tag-only (4 bytes)
            // for simple enums that weren't added to the map.
            sizes
                .get(name.as_str())
                .copied()
                .unwrap_or(TypeLayout { size: 4, align: 4 })
        }
        AirType::Array(inner, n) => {
            let el = resolved_layout(inner, sizes);
            TypeLayout {
                size: el.size * (*n as u32),
                align: el.align,
            }
        }
        other => layout_of(other),
    }
}

fn struct_layout(
    def: &AirStructDef,
    resolved: &HashMap<String, TypeLayout>,
) -> (TypeLayout, Vec<u32>) {
    let mut offset: u32 = 0;
    let mut max_align: u32 = 1;
    let mut offsets = Vec::with_capacity(def.fields.len());

    for field in &def.fields {
        let fl = resolved_layout(&field.ty, resolved);
        offset = align_to(offset, fl.align);
        offsets.push(offset);
        offset += fl.size;
        max_align = max_align.max(fl.align);
    }

    let total = TypeLayout {
        size: align_to(offset, max_align),
        align: max_align,
    };
    (total, offsets)
}

fn align_to(offset: u32, align: u32) -> u32 {
    (offset + align - 1) & !(align - 1)
}

fn detect_self_references(structs: &[AirStructDef]) {
    for def in structs {
        for field in &def.fields {
            if references_by_value(&field.ty, &def.name) {
                panic!(
                    "struct `{}` has infinite size: field `{}` contains `{}` by value",
                    def.name, field.name, def.name
                );
            }
        }
    }
}

fn references_by_value(ty: &AirType, target: &str) -> bool {
    match ty {
        AirType::Struct(name) => name == target,
        AirType::Array(inner, _) => references_by_value(inner, target),
        _ => false,
    }
}

fn field_struct_deps(ty: &AirType, deps: &mut HashSet<String>) {
    match ty {
        AirType::Struct(name) => {
            deps.insert(name.clone());
        }
        AirType::Array(inner, _) => field_struct_deps(inner, deps),
        _ => {}
    }
}

fn topological_order(structs: &[AirStructDef], name_to_idx: &HashMap<String, usize>) -> Vec<usize> {
    let n = structs.len();
    let mut in_degree = vec![0u32; n];
    let mut dependents: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, def) in structs.iter().enumerate() {
        let mut deps = HashSet::new();
        for field in &def.fields {
            field_struct_deps(&field.ty, &mut deps);
        }
        deps.remove(&def.name);
        for dep in deps {
            if let Some(&dep_idx) = name_to_idx.get(&dep) {
                dependents[dep_idx].push(i);
                in_degree[i] += 1;
            }
        }
    }

    let mut queue: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);

    while let Some(node) = queue.pop() {
        order.push(node);
        for &dep in &dependents[node] {
            in_degree[dep] -= 1;
            if in_degree[dep] == 0 {
                queue.push(dep);
            }
        }
    }

    if order.len() != n {
        let cycle: Vec<&str> = (0..n)
            .filter(|&i| in_degree[i] > 0)
            .map(|i| structs[i].name.as_str())
            .collect();
        panic!("recursive struct cycle: {}", cycle.join(" <-> "));
    }

    order
}

/// Returns true if the enum has any data variants (non-empty payload).
pub fn enum_has_data(def: &AirEnumDef) -> bool {
    def.variants.iter().any(|v| !v.payload.is_empty())
}

/// Compute the max alignment needed across all payload fields of a data enum.
fn enum_max_payload_align(
    def: &AirEnumDef,
    sizes: &HashMap<String, TypeLayout>,
) -> u32 {
    def.variants
        .iter()
        .flat_map(|v| v.payload.iter())
        .map(|ty| resolved_layout(ty, sizes).align)
        .max()
        .unwrap_or(1)
}

/// Compute the max payload size in bytes across all variants of a data enum.
/// Each variant's payload is laid out with proper alignment padding between fields,
/// matching the aligned offsets that codegen uses when storing fields.
///
/// `struct_sizes` must contain computed sizes for any struct types that appear
/// in enum variant payloads. Pass `&program.struct_sizes` after `compute_layouts`.
pub fn enum_max_payload_size(
    def: &AirEnumDef,
    struct_sizes: &HashMap<String, TypeLayout>,
) -> u32 {
    def.variants
        .iter()
        .map(|v| {
            let mut offset = 0u32;
            for ty in &v.payload {
                let layout = resolved_layout(ty, struct_sizes);
                offset = (offset + layout.align - 1) & !(layout.align - 1);
                offset += layout.size;
            }
            offset
        })
        .max()
        .unwrap_or(0)
}
