#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuntimeVariant {
    Leak,
    #[default]
    Rc,
    RcCycles,
}

impl RuntimeVariant {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "leak" => Some(Self::Leak),
            "rc" => Some(Self::Rc),
            "rc+cycles" => Some(Self::RcCycles),
            _ => None,
        }
    }

    pub fn lib_suffix(&self) -> &'static str {
        match self {
            Self::Leak => "leak",
            Self::Rc => "rc",
            Self::RcCycles => "rc-cycles",
        }
    }
}
