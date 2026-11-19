use super::{
    ModuleFront, Typed, backend_diagnostic_error, build_imports, exports_of, fall_back_to_prelude,
    fallback_source_span, known_globals,
};
use crate::modules::ModuleUnit;
use aelys_air::{AirEnumDef, AirFunction, AirStructDef};
use aelys_common::error::{AelysError, Fault};
use aelys_sema::localize::{erase_type_vars, localize_imports};
use aelys_sema::modules::qualify_value;
use aelys_sema::{ModuleExports, ModuleImports, TypeInference, TypedProgram};
use std::collections::HashSet;
use std::sync::Arc;

pub(super) fn declare(
    units: &[ModuleUnit],
    first: usize,
    exports: &mut Vec<Arc<ModuleExports>>,
    prelude: Option<&ModuleImports>,
) -> Result<(), AelysError> {
    exports.extend(units[first..].iter().map(|unit| {
        Arc::new(ModuleExports {
            path: unit.dotted.clone(),
            ..Default::default()
        })
    }));
    let items: usize = units[first..].iter().map(|u| u.stmts.len()).sum();
    let rounds = items + 2;
    for _ in 0..rounds {
        let mut settled = true;
        for (index, unit) in units.iter().enumerate().skip(first) {
            let mut imports = build_imports(unit, index, exports, true)?;
            if let Some(scope) = prelude {
                fall_back_to_prelude(&mut imports, scope, &unit.bound_types, &unit.bound_values);
            }
            localize_imports(&mut imports);
            let program = TypeInference::infer_declarations(
                unit.stmts.clone(),
                unit.source.clone(),
                imports,
                known_globals(),
            );
            let mut declared = exports_of(unit, &program)?;
            erase_type_vars(&mut declared);
            if *exports[index] != declared {
                settled = false;
                exports[index] = Arc::new(declared);
            }
        }
        if settled {
            return Ok(());
        }
    }
    let root = &units.last().expect("a group holds its root").source;
    Err(backend_diagnostic_error(
        root.clone(),
        fallback_source_span(root.as_ref()),
        "modules",
        format!("the declarations of a module cycle did not settle in {rounds} rounds"),
        None,
        None,
        Fault::Compiler,
    ))
}

pub(super) fn settle_effects(
    units: &[ModuleUnit],
    typed: &[Typed],
    imports: &mut aelys_air::bir::Imports,
) {
    let programs: Vec<&TypedProgram> = typed.iter().map(|t| &t.inference.program).collect();
    for (unit, program) in units.iter().zip(&programs) {
        for name in aelys_air::lower::retaining_declarations(program) {
            imports
                .effects
                .entry(qualify_value(&unit.dotted, &name))
                .or_insert(aelys_air::bir::EffectSet::EMPTY);
        }
    }
    loop {
        let mut changed = false;
        for (unit, program) in units.iter().zip(&programs) {
            for (name, set) in aelys_air::bir::effect_summary(program, imports) {
                let name = qualify_value(&unit.dotted, &name);
                changed |= imports.effects.insert(name, set) != Some(set);
            }
        }
        if !changed {
            break;
        }
    }
}

// neighbours' functions are assumed to return fresh values until their own lowering proves it
pub(super) struct GroupLowering {
    pub(super) start: usize,
    structs: Vec<Vec<AirStructDef>>,
    enums: Vec<Vec<AirEnumDef>>,
    pub(super) assumed: HashSet<String>,
    own: Vec<HashSet<String>>,
    unproven: HashSet<String>,
}

impl GroupLowering {
    pub(super) fn new(fronts: &[ModuleFront], typed: &[&TypedProgram], start: usize) -> Self {
        let mut group = GroupLowering {
            start,
            structs: Vec::new(),
            enums: Vec::new(),
            assumed: HashSet::new(),
            own: Vec::new(),
            unproven: HashSet::new(),
        };
        for (front, program) in fronts.iter().zip(typed).skip(start) {
            let (mut structs, mut enums) = aelys_air::lower::lower_type_decls(program);
            aelys_air::modules::qualify_type_decls(&mut structs, &mut enums, &front.dotted);
            group.structs.push(structs);
            group.enums.push(enums);
            let own: HashSet<String> = aelys_air::lower::retaining_declarations(program)
                .iter()
                .map(|name| qualify_value(&front.dotted, name))
                .collect();
            group.assumed.extend(own.iter().cloned());
            group.own.push(own);
        }
        group
    }

    pub(super) fn assumed_by(&self, index: usize) -> impl Iterator<Item = String> + '_ {
        let own = &self.own[index - self.start];
        self.assumed
            .iter()
            .filter(move |name| !own.contains(*name))
            .cloned()
    }

    pub(super) fn structs_after(&self, index: usize) -> impl Iterator<Item = AirStructDef> + '_ {
        self.structs
            .iter()
            .skip(index + 1 - self.start)
            .flatten()
            .cloned()
    }

    pub(super) fn enums_after(&self, index: usize) -> impl Iterator<Item = AirEnumDef> + '_ {
        self.enums
            .iter()
            .skip(index + 1 - self.start)
            .flatten()
            .cloned()
    }

    pub(super) fn record_unproven(
        &mut self,
        functions: &[AirFunction],
        carriers: &aelys_air::counts::Carriers<'_>,
        proven: &HashSet<String>,
    ) {
        self.unproven.extend(
            functions
                .iter()
                .filter(|f| !f.is_extern && carriers.counted(&f.ret_ty))
                .filter(|f| self.assumed.contains(&f.name) && !proven.contains(&f.name))
                .map(|f| f.name.clone()),
        );
    }

    pub(super) fn retract_unproven(&mut self) -> bool {
        if self.unproven.is_empty() {
            return false;
        }
        for name in self.unproven.drain() {
            self.assumed.remove(&name);
        }
        true
    }
}
