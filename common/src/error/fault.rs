#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fault {
    Compiler,
    Unsupported,
    Program,
    Environment,
}

impl Fault {
    // a helper that reads the first code of a rendering must land on the gravest class
    pub const JOIN_ORDER: [Fault; 4] = [
        Fault::Compiler,
        Fault::Program,
        Fault::Unsupported,
        Fault::Environment,
    ];

    pub fn code(self) -> u16 {
        match self {
            Fault::Compiler => 901,
            Fault::Unsupported => 902,
            Fault::Environment => 903,
            Fault::Program => 904,
        }
    }

    pub fn annotation(self) -> &'static str {
        match self {
            Fault::Compiler => "compiler bug",
            Fault::Unsupported => "not supported yet",
            Fault::Environment => "the toolchain refused",
            Fault::Program => "not well formed for the backend",
        }
    }
}
