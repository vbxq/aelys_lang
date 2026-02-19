use super::WarningKind;

impl WarningKind {
    pub fn message(&self, ctx: Option<&str>) -> String {
        let name = ctx.unwrap_or("<unknown>");

        match self {
            Self::InlineRecursive => {
                format!("cannot inline recursive function '{}'", name)
            }

            Self::InlineMutualRecursion { cycle } => {
                format!(
                    "cannot inline '{}' due to mutual recursion: {}",
                    name,
                    cycle.join(" → ")
                )
            }

            Self::InlineHasCaptures => {
                format!(
                    "function '{}' captures variables, inlining may change semantics",
                    name
                )
            }

            Self::InlinePublicFunction => {
                format!(
                    "inlining public function '{}', original will be kept for external callers",
                    name
                )
            }

            Self::InlineNativeFunction => {
                format!("cannot inline native function '{}'", name)
            }

            Self::UnusedVariable { name: var } => {
                format!("unused variable '{}'", var)
            }

            Self::UnusedFunction { name: func } => {
                format!("function '{}' is never called", func)
            }

            Self::UnusedImport { module } => {
                format!("unused import '{}'", module)
            }

            Self::DeprecatedFunction {
                name: func,
                replacement,
            } => match replacement {
                Some(r) => format!("'{}' is deprecated, use '{}' instead", func, r),
                None => format!("'{}' is deprecated", func),
            },

            Self::ShadowedVariable { name: var } => {
                format!("variable '{}' shadows a previous binding", var)
            }

            Self::UnknownType { name: ty } => {
                format!("unknown type '{}', treating as dynamic", ty)
            }

            Self::UnknownTypeParameter { param, in_type } => {
                format!(
                    "unknown type parameter '{}' in {}<...>, treating as dynamic",
                    param, in_type
                )
            }

            Self::IncompatibleComparison { left, right, op } => {
                format!(
                    "comparing {} {} {} will always be {}",
                    left,
                    op,
                    right,
                    if op == "!=" { "true" } else { "false" }
                )
            }
        }
    }
}
