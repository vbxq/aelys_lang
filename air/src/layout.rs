use crate::{AirProgram, AirStructDef, AirType};
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
        AirType::Ptr(_) | AirType::Str | AirType::FnPtr { .. } => TypeLayout { size: 8, align: 8 },
        AirType::Void => TypeLayout { size: 0, align: 1 },
        AirType::Slice(_) => TypeLayout { size: 16, align: 8 },
        AirType::Param(_) => TypeLayout { size: 8, align: 8 },
        AirType::Array(inner, n) => {
            let el = layout_of(inner);
            TypeLayout {
                size: el.size * (*n as u32),
                align: el.align,
            }
        }
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
}

fn resolved_layout(ty: &AirType, structs: &HashMap<String, TypeLayout>) -> TypeLayout {
    match ty {
        AirType::Struct(name) => *structs
            .get(name.as_str())
            .unwrap_or_else(|| panic!("struct `{name}` referenced before its layout is computed")),
        AirType::Array(inner, n) => {
            let el = resolved_layout(inner, structs);
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
