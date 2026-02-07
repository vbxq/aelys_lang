use super::WarningKind;

impl WarningKind {
    pub fn annotation(&self) -> &'static str {
        match self {
            Self::InlineRecursive => "recursive call here",
            Self::InlineMutualRecursion { .. } => "part of mutual recursion",
            Self::InlineHasCaptures => "captures from outer scope",
            Self::InlinePublicFunction => "public function",
            Self::InlineNativeFunction => "native, no body to inline",
            Self::UnusedVariable { .. } => "never used",
            Self::UnusedFunction { .. } => "never called",
            Self::UnusedImport { .. } => "never used",
            Self::DeprecatedFunction { .. } => "deprecated",
            Self::ShadowedVariable { .. } => "shadows earlier binding",
            Self::UnknownType { .. } => "unknown type name",
            Self::UnknownTypeParameter { .. } => "unknown type parameter",
        }
    }

    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::InlineRecursive => Some("remove @inline from this function"),
            Self::InlineMutualRecursion { .. } => Some("break the cycle or remove @inline"),
            Self::InlineHasCaptures => Some("use @inline_always if this is intentional"),
            Self::InlinePublicFunction => None,
            Self::InlineNativeFunction => Some("remove @inline from this declaration"),
            Self::UnusedVariable { .. } => Some("prefix with _ to silence"),
            Self::UnusedFunction { .. } => Some("remove it or mark as pub"),
            Self::UnusedImport { .. } => Some("remove the import"),
            Self::DeprecatedFunction { .. } => None,
            Self::ShadowedVariable { .. } => Some("use a different name"),
            Self::UnknownType { .. } => Some("use int, float, bool, string, array, or vec"),
            Self::UnknownTypeParameter { .. } => Some("use a known type like int, float, string"),
        }
    }

    pub fn note(&self) -> Option<&'static str> {
        match self {
            Self::InlineRecursive => Some("inlining would cause infinite expansion"),
            Self::InlineHasCaptures => {
                Some("captured variables may behave differently when inlined")
            }
            Self::InlineNativeFunction => Some("native functions have no Aelys body"),
            Self::InlinePublicFunction => {
                Some("callers from other modules will still use the non-inlined version")
            }
            _ => None,
        }
    }
}
