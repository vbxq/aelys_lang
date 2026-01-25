use super::TypeVarId;
use std::fmt;

/// Type used during inference - may contain unresolved type variables
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InferType {
    // Concrete types
    Int,
    Float,
    Bool,
    String,
    Null,

    // Function type: (params) -> return
    Function {
        params: Vec<InferType>,
        ret: Box<InferType>,
    },

    // Composite types (for future extension)
    Array(Box<InferType>),
    Tuple(Vec<InferType>),

    // Type variable (placeholder during inference)
    Var(TypeVarId),

    // Dynamic type (gradual typing - unifies with anything)
    Dynamic,
}

impl InferType {
    /// Check if this type contains any type variables
    pub fn has_vars(&self) -> bool {
        match self {
            InferType::Var(_) => true,
            InferType::Function { params, ret } => {
                params.iter().any(|p| p.has_vars()) || ret.has_vars()
            }
            InferType::Array(inner) => inner.has_vars(),
            InferType::Tuple(elems) => elems.iter().any(|e| e.has_vars()),
            _ => false,
        }
    }

    /// Check if this type is fully resolved (no variables)
    pub fn is_resolved(&self) -> bool {
        !self.has_vars()
    }

    /// Check if this is a concrete numeric type
    pub fn is_numeric(&self) -> bool {
        matches!(self, InferType::Int | InferType::Float)
    }

    /// Convert from type annotation string
    pub fn from_annotation(name: &str) -> Self {
        match name {
            "int" => InferType::Int,
            "float" => InferType::Float,
            "bool" => InferType::Bool,
            "string" => InferType::String,
            "null" => InferType::Null,
            _ => InferType::Dynamic,
        }
    }

    /// Extract the type variable ID if this is a Var
    pub fn as_var_id(&self) -> Option<TypeVarId> {
        match self {
            InferType::Var(id) => Some(*id),
            _ => None,
        }
    }
}

impl fmt::Display for InferType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InferType::Int => write!(f, "int"),
            InferType::Float => write!(f, "float"),
            InferType::Bool => write!(f, "bool"),
            InferType::String => write!(f, "string"),
            InferType::Null => write!(f, "null"),
            InferType::Function { params, ret } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            InferType::Array(inner) => write!(f, "[{}]", inner),
            InferType::Tuple(elems) => {
                write!(f, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", e)?;
                }
                write!(f, ")")
            }
            InferType::Var(id) => write!(f, "{}", id),
            InferType::Dynamic => write!(f, "dynamic"),
        }
    }
}
