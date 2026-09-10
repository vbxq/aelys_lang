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

    /// never is a subtype of every type `unify(never, t)` always succeeds
    Never,

    Function {
        params: Vec<InferType>,
        ret: Box<InferType>,
        nogc: bool,
    },

    Array(Box<InferType>, Option<u64>),
    Vec(Box<InferType>),
    Rc(Box<InferType>),
    // stage 1 borrows, erased to a raw ptr / fat {ptr,len} in air
    Ref {
        referent: Box<InferType>,
        mutable: bool,
    },
    Slice {
        elem: Box<InferType>,
        mutable: bool,
    },
    Tuple(Vec<InferType>),
    Range,

    Struct(std::string::String),
    Enum(std::string::String, Vec<InferType>),

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
            InferType::Function { params, ret, .. } => {
                params.iter().any(|p| p.has_vars()) || ret.has_vars()
            }
            InferType::Array(inner, _) | InferType::Vec(inner) | InferType::Rc(inner) => {
                inner.has_vars()
            }
            InferType::Ref { referent, .. } => referent.has_vars(),
            InferType::Slice { elem, .. } => elem.has_vars(),
            InferType::Tuple(elems) => elems.iter().any(|e| e.has_vars()),
            InferType::Enum(_, args) => args.iter().any(|a| a.has_vars()),
            _ => false,
        }
    }

    pub fn is_resolved(&self) -> bool {
        !self.has_vars()
    }

    pub fn is_concrete(&self) -> bool {
        match self {
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
            | InferType::Struct(_) => true,
            InferType::Enum(_, args) => args.iter().all(|a| a.is_concrete()),
            InferType::Ref { referent, .. } => referent.is_concrete(),
            InferType::Slice { elem, .. } => elem.is_concrete(),
            _ => false,
        }
    }

    pub fn is_rc(&self) -> bool {
        matches!(self, InferType::Rc(_))
    }

    pub fn contains_rc(&self) -> bool {
        match self {
            InferType::Rc(_) => true,
            InferType::Array(inner, _) | InferType::Vec(inner) => inner.contains_rc(),
            InferType::Ref { referent, .. } => referent.contains_rc(),
            InferType::Slice { elem, .. } => elem.contains_rc(),
            InferType::Tuple(elems) => elems.iter().any(|e| e.contains_rc()),
            InferType::Enum(_, args) => args.iter().any(|a| a.contains_rc()),
            InferType::Function { params, ret, .. } => {
                params.iter().any(|p| p.contains_rc()) || ret.contains_rc()
            }
            _ => false,
        }
    }

    pub fn from_annotation(ann: &aelys_syntax::TypeAnnotation) -> Self {
        if let Some(kind) = ann.reference {
            let mutable = matches!(kind, aelys_syntax::RefKind::Mut);
            if ann.is_slice {
                let elem = ann
                    .type_param
                    .as_ref()
                    .map(|p| Self::from_annotation(p))
                    .unwrap_or(InferType::Dynamic);
                return InferType::Slice {
                    elem: Box::new(elem),
                    mutable,
                };
            }
            let mut base = ann.clone();
            base.reference = None;
            return InferType::Ref {
                referent: Box::new(Self::from_annotation(&base)),
                mutable,
            };
        }
        if ann.is_function_type() {
            let params = ann
                .fn_params
                .as_ref()
                .map(|ps| ps.iter().map(Self::from_annotation).collect())
                .unwrap_or_default();
            let ret = ann
                .fn_ret
                .as_ref()
                .map(|r| Self::from_annotation(r))
                .unwrap_or(InferType::Null);
            return InferType::Function {
                params,
                ret: Box::new(ret),
                nogc: ann.nogc,
            };
        }
        if ann.name == "Rc" {
            let inner = ann
                .type_param
                .as_ref()
                .map(|p| Self::from_annotation(p))
                .unwrap_or(InferType::Dynamic);
            return InferType::Rc(Box::new(inner));
        }
        if ann.name.chars().next().is_some_and(|c| c.is_uppercase()) {
            return InferType::Struct(ann.name.clone());
        }
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
            "string" | "str" => InferType::String,
            "null" | "void" => InferType::Null,
            "array" if ann.array_size.is_some() => {
                let inner = ann
                    .type_param
                    .as_ref()
                    .map(|p| Self::from_annotation(p))
                    .unwrap_or(InferType::Dynamic);
                InferType::Array(Box::new(inner), ann.array_size)
            }
            "vec" => {
                let inner = ann
                    .type_param
                    .as_ref()
                    .map(|p| Self::from_annotation(p))
                    .unwrap_or(InferType::Dynamic);
                InferType::Vec(Box::new(inner))
            }
            _ => InferType::Dynamic,
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
            "string" | "str" => InferType::String,
            "null" | "void" => InferType::Null,
            _ => {
                if name.chars().next().is_some_and(|c| c.is_uppercase()) {
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

    pub fn int_fits(value: i64, ty: &InferType) -> bool {
        match ty {
            InferType::I8 => i8::try_from(value).is_ok(),
            InferType::I16 => i16::try_from(value).is_ok(),
            InferType::I32 => i32::try_from(value).is_ok(),
            InferType::I64 => true,
            InferType::U8 => u8::try_from(value).is_ok(),
            InferType::U16 => u16::try_from(value).is_ok(),
            InferType::U32 => u32::try_from(value).is_ok(),
            InferType::U64 => value >= 0,
            _ => false,
        }
    }

    pub fn float_fits(value: f64, ty: &InferType) -> bool {
        match ty {
            InferType::F32 => value.is_finite() && value.abs() <= f32::MAX as f64,
            InferType::F64 => true,
            _ => false,
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

    fn numeric_rank(&self) -> Option<(i16, bool)> {
        match self {
            InferType::I8 => Some((8, true)),
            InferType::I16 => Some((16, true)),
            InferType::I32 => Some((32, true)),
            InferType::I64 => Some((64, true)),
            InferType::U8 => Some((8, false)),
            InferType::U16 => Some((16, false)),
            InferType::U32 => Some((32, false)),
            InferType::U64 => Some((64, false)),
            InferType::F32 => Some((-32, true)),
            InferType::F64 => Some((-64, true)),
            _ => None,
        }
    }

    pub fn can_implicit_widen_to(&self, target: &InferType) -> bool {
        if self == target {
            return false;
        }

        let (src_rank, src_signed) = match self.numeric_rank() {
            Some(r) => r,
            None => return false,
        };

        let (tgt_rank, tgt_signed) = match target.numeric_rank() {
            Some(r) => r,
            None => return false,
        };

        if src_rank < 0 {
            return false;
        }

        if tgt_rank < 0 {
            let tgt_bits = -tgt_rank; // 32 or 64
            return if tgt_bits == 32 {
                src_rank <= 16
            } else {
                src_rank <= 32
            };
        }

        if src_signed && tgt_signed {
            tgt_rank > src_rank
        } else if !src_signed && !tgt_signed {
            tgt_rank > src_rank
        } else if !src_signed && tgt_signed {
            tgt_rank > src_rank
        } else {
            false
        }
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
            InferType::Never => write!(f, "!"),
            InferType::Function { params, ret, .. } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            InferType::Array(inner, Some(n)) => write!(f, "[{}; {}]", inner, n),
            InferType::Array(inner, None) => write!(f, "[{}]", inner),
            InferType::Vec(inner) => write!(f, "vec[{}]", inner),
            InferType::Rc(inner) => write!(f, "Rc<{}>", inner),
            InferType::Ref { referent, mutable } => {
                write!(f, "&{}{}", if *mutable { "mut " } else { "" }, referent)
            }
            InferType::Slice { elem, mutable } => {
                write!(f, "&{}[{}]", if *mutable { "mut " } else { "" }, elem)
            }
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
            InferType::Struct(name) => {
                write!(f, "{}", crate::modules::strip_type_head(name))
            }
            InferType::Enum(name, type_args) => {
                write!(f, "{}", crate::modules::strip_type_head(name))?;
                if !type_args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            InferType::Var(id) => write!(f, "{}", id),
            InferType::Dynamic => write!(f, "dynamic"),
        }
    }
}
