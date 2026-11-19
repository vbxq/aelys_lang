use crate::lowering::functions::function_symbol_name;
use aelys_air::{AirFunction, AirGlobal, AirProgram, FunctionId};
use std::collections::HashMap;

// each map keeps the first entry for a key
pub(crate) struct ProgramIndex<'a> {
    pub(crate) symbols: HashMap<FunctionId, String>,
    by_id: HashMap<FunctionId, &'a AirFunction>,
    by_name: HashMap<&'a str, &'a AirFunction>,
    externs: HashMap<&'a str, &'a AirFunction>,
    globals: HashMap<&'a str, &'a AirGlobal>,
}

impl<'a> ProgramIndex<'a> {
    pub(crate) fn new(program: &'a AirProgram) -> Self {
        let mut index = ProgramIndex {
            symbols: HashMap::with_capacity(program.functions.len()),
            by_id: HashMap::new(),
            by_name: HashMap::new(),
            externs: HashMap::new(),
            globals: HashMap::new(),
        };
        for function in &program.functions {
            index
                .symbols
                .insert(function.id, function_symbol_name(function));
            index.by_id.entry(function.id).or_insert(function);
            index.by_name.entry(&function.name).or_insert(function);
            if function.is_extern {
                index.externs.entry(&function.name).or_insert(function);
            }
        }
        for global in &program.globals {
            index.globals.entry(&global.name).or_insert(global);
        }
        index
    }

    pub(crate) fn function(&self, id: FunctionId) -> Option<&'a AirFunction> {
        self.by_id.get(&id).copied()
    }

    pub(crate) fn named(&self, name: &str) -> Option<&'a AirFunction> {
        self.by_name.get(name).copied()
    }

    pub(crate) fn external(&self, name: &str) -> Option<&'a AirFunction> {
        self.externs.get(name).copied()
    }

    pub(crate) fn global(&self, name: &str) -> Option<&'a AirGlobal> {
        self.globals.get(name).copied()
    }
}
