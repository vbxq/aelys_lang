use super::{ModuleFront, Typed, build_imports, fall_back_to_prelude, known_globals};
use crate::modules::ModuleUnit;
use aelys_air::{AirEnumDef, AirFunction, AirStructDef};
use aelys_common::error::AelysError;
use aelys_sema::localize::{erase_type_vars, localize_imports};
use aelys_sema::modules::{collect_exports, qualify_value};
use aelys_sema::{ModuleExports, ModuleImports, TypeInference, TypedProgram};
use std::collections::HashSet;
use std::sync::Arc;

// each round settles one more link of a const or trait chain that crosses modules
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
    let mut prints = vec![String::new(); units.len() - first];
    for _ in 0..=prints.len() + 1 {
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
            let mut declared = collect_exports(&unit.dotted, &program);
            erase_type_vars(&mut declared);
            let print = fingerprint(&declared);
            if print != prints[index - first] {
                settled = false;
                prints[index - first] = print;
            }
            exports[index] = Arc::new(declared);
        }
        if settled {
            break;
        }
    }
    Ok(())
}

// the effects of every user function as one least fixpoint, so a nogc call may cross a cycle
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

fn fingerprint(exports: &ModuleExports) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.extend(exports.values.iter().map(|(name, v)| format!("{name} {v:?}")));
    lines.extend(exports.types.iter().map(|(name, t)| format!("{name} {t:?}")));
    lines.extend(exports.methods.iter().map(|(ty, e)| format!("{ty} {e:?}")));
    lines.extend(exports.traits.iter().map(|(name, t)| format!("{name} {t:?}")));
    lines.extend(exports.impls.iter().map(|imp| format!("{imp:?}")));
    lines.sort();
    lines.join("\n")
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

    // a module proves its own functions itself, only its neighbours' are taken on trust
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

    // true when the group has to be lowered again without the failed assumptions
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
