// -O0 through -O3, classic style
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum OptimizationLevel {
    None,  // -O0
    Basic, // -O1: just constant folding
    #[default]
    Standard, // -O2: folding + DCE + unused vars
    Aggressive, // -O3: same as O2 for now (TODO: more passes)
}

impl OptimizationLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "0" | "none" => Some(Self::None),
            "1" | "basic" => Some(Self::Basic),
            "2" | "standard" => Some(Self::Standard),
            "3" | "aggressive" => Some(Self::Aggressive),
            _ => None,
        }
    }

    pub fn llvm_pass_pipeline(&self) -> &'static str {
        match self {
            Self::None => "default<O0>",
            Self::Basic => "default<O1>",
            Self::Standard => "default<O2>",
            Self::Aggressive => "default<O3>",
        }
    }

    pub fn numeric(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Basic => 1,
            Self::Standard => 2,
            Self::Aggressive => 3,
        }
    }
}
