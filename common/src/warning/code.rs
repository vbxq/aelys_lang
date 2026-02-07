use super::WarningKind;

impl WarningKind {
    pub fn code(&self) -> u16 {
        match self {
            // inline warnings: 100-199
            Self::InlineRecursive => 101,
            Self::InlineMutualRecursion { .. } => 102,
            Self::InlineHasCaptures => 103,
            Self::InlinePublicFunction => 104,
            Self::InlineNativeFunction => 105,

            // unused: 200-299
            Self::UnusedVariable { .. } => 201,
            Self::UnusedFunction { .. } => 202,
            Self::UnusedImport { .. } => 203,

            // deprecation: 300-399
            Self::DeprecatedFunction { .. } => 301,

            // style: 400-499
            Self::ShadowedVariable { .. } => 401,

            // type: 500-599
            Self::UnknownType { .. } => 501,
            Self::UnknownTypeParameter { .. } => 502,
        }
    }
}
