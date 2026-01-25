// -O0 through -O3, classic style
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptimizationLevel {
    None,                // -O0
    Basic,               // -O1: just constant folding
    #[default]
    Standard,            // -O2: folding + DCE + unused vars
    Aggressive,          // -O3: same as O2 for now (TODO: more passes)
}

impl OptimizationLevel {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "0" | "none" => Some(Self::None),
            "1" | "basic" => Some(Self::Basic),
            "2" | "standard" => Some(Self::Standard),
            "3" | "aggressive" => Some(Self::Aggressive),
            _ => None,
        }
    }
}
