use super::InferType;
use std::fmt;

// resolved type, after inference all veriable are resolved
// this is what the compiler uses for opcode selection
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolvedType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    String,
    Null,

    Function {
        params: Vec<ResolvedType>,
        ret: Box<ResolvedType>,
    },

    Array(Box<ResolvedType>),
    Vec(Box<ResolvedType>),
    Tuple(Vec<ResolvedType>),
    Range,

    Struct(std::string::String),

    Dynamic,

    Uncertain(Box<ResolvedType>),
}

impl ResolvedType {
    pub fn is_certain(&self) -> bool {
        !matches!(self, ResolvedType::Dynamic | ResolvedType::Uncertain(_))
    }

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            ResolvedType::I8
                | ResolvedType::I16
                | ResolvedType::I32
                | ResolvedType::I64
                | ResolvedType::U8
                | ResolvedType::U16
                | ResolvedType::U32
                | ResolvedType::U64
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, ResolvedType::F32 | ResolvedType::F64)
    }

    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    pub fn is_integer_ish(&self) -> bool {
        match self {
            t if t.is_integer() => true,
            ResolvedType::Uncertain(inner) => inner.is_integer(),
            _ => false,
        }
    }

    pub fn is_float_ish(&self) -> bool {
        match self {
            t if t.is_float() => true,
            ResolvedType::Uncertain(inner) => inner.is_float(),
            _ => false,
        }
    }

    pub fn needs_guard(&self) -> bool {
        matches!(self, ResolvedType::Uncertain(_))
    }

    pub fn unwrap_uncertain(&self) -> &ResolvedType {
        match self {
            ResolvedType::Uncertain(inner) => inner,
            other => other,
        }
    }

    pub fn from_infer_type(ty: &InferType) -> Self {
        match ty {
            InferType::I8 => ResolvedType::I8,
            InferType::I16 => ResolvedType::I16,
            InferType::I32 => ResolvedType::I32,
            InferType::I64 => ResolvedType::I64,
            InferType::U8 => ResolvedType::U8,
            InferType::U16 => ResolvedType::U16,
            InferType::U32 => ResolvedType::U32,
            InferType::U64 => ResolvedType::U64,
            InferType::F32 => ResolvedType::F32,
            InferType::F64 => ResolvedType::F64,
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
            InferType::Vec(inner) => {
                ResolvedType::Vec(Box::new(ResolvedType::from_infer_type(inner)))
            }
            InferType::Tuple(elems) => {
                ResolvedType::Tuple(elems.iter().map(ResolvedType::from_infer_type).collect())
            }
            InferType::Range => ResolvedType::Range,
            InferType::Struct(name) => ResolvedType::Struct(name.clone()),
            InferType::Var(_) => ResolvedType::Dynamic,
            InferType::Dynamic => ResolvedType::Dynamic,
        }
    }
}

impl fmt::Display for ResolvedType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolvedType::I8 => write!(f, "i8"),
            ResolvedType::I16 => write!(f, "i16"),
            ResolvedType::I32 => write!(f, "i32"),
            ResolvedType::I64 => write!(f, "i64"),
            ResolvedType::U8 => write!(f, "u8"),
            ResolvedType::U16 => write!(f, "u16"),
            ResolvedType::U32 => write!(f, "u32"),
            ResolvedType::U64 => write!(f, "u64"),
            ResolvedType::F32 => write!(f, "f32"),
            ResolvedType::F64 => write!(f, "f64"),
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
            ResolvedType::Vec(inner) => write!(f, "vec[{}]", inner),
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
            ResolvedType::Range => write!(f, "range"),
            ResolvedType::Struct(name) => write!(f, "{}", name),
            ResolvedType::Dynamic => write!(f, "dynamic"),
            ResolvedType::Uncertain(inner) => write!(f, "?{}", inner),
        }
    }
}
