use super::InferType;
use std::fmt;

/// Resolved type - after inference, all variables resolved
/// This is what the compiler uses for opcode selection
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolvedType {
    // Concrete types
    Int,
    Float,
    Bool,
    String,
    Null,

    // Function type
    Function {
        params: Vec<ResolvedType>,
        ret: Box<ResolvedType>,
    },

    // Composite types
    Array(Box<ResolvedType>),
    Tuple(Vec<ResolvedType>),

    // Dynamic - couldn't infer, use generic opcodes
    Dynamic,

    // Uncertain - probably this type but came through dynamic path
    // Use guarded opcodes
    Uncertain(Box<ResolvedType>),
}

impl ResolvedType {
    /// Check if this type is certain (not Dynamic or Uncertain)
    pub fn is_certain(&self) -> bool {
        !matches!(self, ResolvedType::Dynamic | ResolvedType::Uncertain(_))
    }

    /// Check if this is Int or Uncertain(Int)
    pub fn is_int_ish(&self) -> bool {
        match self {
            ResolvedType::Int => true,
            ResolvedType::Uncertain(inner) => **inner == ResolvedType::Int,
            _ => false,
        }
    }

    /// Check if this is Float or Uncertain(Float)
    pub fn is_float_ish(&self) -> bool {
        match self {
            ResolvedType::Float => true,
            ResolvedType::Uncertain(inner) => **inner == ResolvedType::Float,
            _ => false,
        }
    }

    /// Check if this needs a guard (is Uncertain)
    pub fn needs_guard(&self) -> bool {
        matches!(self, ResolvedType::Uncertain(_))
    }

    /// Unwrap Uncertain to get inner type
    pub fn unwrap_uncertain(&self) -> &ResolvedType {
        match self {
            ResolvedType::Uncertain(inner) => inner,
            other => other,
        }
    }

    /// Convert from InferType after substitution
    /// Any remaining Var becomes Dynamic
    pub fn from_infer_type(ty: &InferType) -> Self {
        match ty {
            InferType::Int => ResolvedType::Int,
            InferType::Float => ResolvedType::Float,
            InferType::Bool => ResolvedType::Bool,
            InferType::String => ResolvedType::String,
            InferType::Null => ResolvedType::Null,
            InferType::Function { params, ret } => ResolvedType::Function {
                params: params.iter().map(ResolvedType::from_infer_type).collect(),
                ret: Box::new(ResolvedType::from_infer_type(ret)),
            },
            InferType::Array(inner) => {
                ResolvedType::Array(Box::new(ResolvedType::from_infer_type(inner)))
            }
            InferType::Tuple(elems) => {
                ResolvedType::Tuple(elems.iter().map(ResolvedType::from_infer_type).collect())
            }
            InferType::Var(_) => ResolvedType::Dynamic,
            InferType::Dynamic => ResolvedType::Dynamic,
        }
    }
}

impl fmt::Display for ResolvedType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolvedType::Int => write!(f, "int"),
            ResolvedType::Float => write!(f, "float"),
            ResolvedType::Bool => write!(f, "bool"),
            ResolvedType::String => write!(f, "string"),
            ResolvedType::Null => write!(f, "null"),
            ResolvedType::Function { params, ret } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            ResolvedType::Array(inner) => write!(f, "[{}]", inner),
            ResolvedType::Tuple(elems) => {
                write!(f, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", e)?;
                }
                write!(f, ")")
            }
            ResolvedType::Dynamic => write!(f, "dynamic"),
            ResolvedType::Uncertain(inner) => write!(f, "?{}", inner),
        }
    }
}
