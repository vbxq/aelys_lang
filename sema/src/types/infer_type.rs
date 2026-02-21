use super::TypeVarId;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InferType {
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
        params: Vec<InferType>,
        ret: Box<InferType>,
    },

    Array(Box<InferType>),
    Vec(Box<InferType>),
    Tuple(Vec<InferType>),
    Range,

    Struct(std::string::String),

    Var(TypeVarId),

    Dynamic,
}

impl InferType {
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            InferType::I8
                | InferType::I16
                | InferType::I32
                | InferType::I64
                | InferType::U8
                | InferType::U16
                | InferType::U32
                | InferType::U64
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, InferType::F32 | InferType::F64)
    }

    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    pub fn has_vars(&self) -> bool {
        match self {
            InferType::Var(_) => true,
            InferType::Function { params, ret } => {
                params.iter().any(|p| p.has_vars()) || ret.has_vars()
            }
            InferType::Array(inner) | InferType::Vec(inner) => inner.has_vars(),
            InferType::Tuple(elems) => elems.iter().any(|e| e.has_vars()),
            _ => false,
        }
    }

    pub fn is_resolved(&self) -> bool {
        !self.has_vars()
    }

    pub fn is_concrete(&self) -> bool {
        matches!(
            self,
            InferType::I8
                | InferType::I16
                | InferType::I32
                | InferType::I64
                | InferType::U8
                | InferType::U16
                | InferType::U32
                | InferType::U64
                | InferType::F32
                | InferType::F64
                | InferType::Bool
                | InferType::String
                | InferType::Null
                | InferType::Struct(_)
        )
    }

    pub fn from_annotation(ann: &aelys_syntax::TypeAnnotation) -> Self {
        let name_lower = ann.name.to_lowercase();
        match name_lower.as_str() {
            "int" | "i64" | "int64" => InferType::I64,
            "i8" | "int8" => InferType::I8,
            "i16" | "int16" => InferType::I16,
            "i32" | "int32" => InferType::I32,
            "u8" | "uint8" => InferType::U8,
            "u16" | "uint16" => InferType::U16,
            "u32" | "uint32" => InferType::U32,
            "u64" | "uint64" => InferType::U64,
            "float" | "f64" | "float64" => InferType::F64,
            "f32" | "float32" => InferType::F32,
            "bool" => InferType::Bool,
            "string" => InferType::String,
            "null" | "void" => InferType::Null,
            "array" => {
                let inner = ann
                    .type_param
                    .as_ref()
                    .map(|p| Self::from_annotation(p))
                    .unwrap_or(InferType::Dynamic);
                InferType::Array(Box::new(inner))
            }
            "vec" => {
                let inner = ann
                    .type_param
                    .as_ref()
                    .map(|p| Self::from_annotation(p))
                    .unwrap_or(InferType::Dynamic);
                InferType::Vec(Box::new(inner))
            }
            _ => {
                if ann.name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    InferType::Struct(ann.name.clone())
                } else {
                    InferType::Dynamic
                }
            }
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "int" | "i64" | "int64" => InferType::I64,
            "i8" | "int8" => InferType::I8,
            "i16" | "int16" => InferType::I16,
            "i32" | "int32" => InferType::I32,
            "u8" | "uint8" => InferType::U8,
            "u16" | "uint16" => InferType::U16,
            "u32" | "uint32" => InferType::U32,
            "u64" | "uint64" => InferType::U64,
            "float" | "f64" | "float64" => InferType::F64,
            "f32" | "float32" => InferType::F32,
            "bool" => InferType::Bool,
            "string" => InferType::String,
            "null" | "void" => InferType::Null,
            _ => {
                if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    InferType::Struct(name.to_string())
                } else {
                    InferType::Dynamic
                }
            }
        }
    }

    pub fn as_var_id(&self) -> Option<TypeVarId> {
        match self {
            InferType::Var(id) => Some(*id),
            _ => None,
        }
    }

    pub fn all_integer_types() -> Vec<InferType> {
        vec![
            InferType::I8,
            InferType::I16,
            InferType::I32,
            InferType::I64,
            InferType::U8,
            InferType::U16,
            InferType::U32,
            InferType::U64,
        ]
    }

    pub fn all_float_types() -> Vec<InferType> {
        vec![InferType::F32, InferType::F64]
    }

    pub fn all_numeric_types() -> Vec<InferType> {
        let mut types = Self::all_integer_types();
        types.extend(Self::all_float_types());
        types
    }
}

impl fmt::Display for InferType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InferType::I8 => write!(f, "i8"),
            InferType::I16 => write!(f, "i16"),
            InferType::I32 => write!(f, "i32"),
            InferType::I64 => write!(f, "i64"),
            InferType::U8 => write!(f, "u8"),
            InferType::U16 => write!(f, "u16"),
            InferType::U32 => write!(f, "u32"),
            InferType::U64 => write!(f, "u64"),
            InferType::F32 => write!(f, "f32"),
            InferType::F64 => write!(f, "f64"),
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
            InferType::Vec(inner) => write!(f, "vec[{}]", inner),
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
            InferType::Range => write!(f, "range"),
            InferType::Struct(name) => write!(f, "{}", name),
            InferType::Var(id) => write!(f, "{}", id),
            InferType::Dynamic => write!(f, "dynamic"),
        }
    }
}
