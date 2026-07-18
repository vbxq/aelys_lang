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

    /// The bottom type: represents diverging control flow (return, break, etc.).
    /// Never is a subtype of every type — `unify(Never, T)` always succeeds
    /// without constraining T. This is safe because a Never-typed expression
    /// never produces a value, so any expected type is vacuously compatible.
    Never,

    Function {
        params: Vec<InferType>,
        ret: Box<InferType>,
    },

    Array(Box<InferType>, Option<u64>),
    Vec(Box<InferType>),
    // the only reference type, every other constructor is a value type
    Rc(Box<InferType>),
    Tuple(Vec<InferType>),
    Range,

    Struct(std::string::String),
    /// Enum type with optional type arguments for generic enums.
    /// Non-generic enums have an empty Vec.
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
            InferType::Function { params, ret } => {
                params.iter().any(|p| p.has_vars()) || ret.has_vars()
            }
            InferType::Array(inner, _) | InferType::Vec(inner) | InferType::Rc(inner) => {
                inner.has_vars()
            }
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
            // An Enum is only concrete if all its type args are also concrete.
            InferType::Enum(_, args) => args.iter().all(|a| a.is_concrete()),
            _ => false,
        }
    }

    pub fn is_rc(&self) -> bool {
        matches!(self, InferType::Rc(_))
    }

    // blind to nominals: a struct holding an Rc field reads as false here
    pub fn contains_rc(&self) -> bool {
        match self {
            InferType::Rc(_) => true,
            InferType::Array(inner, _) | InferType::Vec(inner) => inner.contains_rc(),
            InferType::Tuple(elems) => elems.iter().any(|e| e.contains_rc()),
            InferType::Enum(_, args) => args.iter().any(|a| a.contains_rc()),
            InferType::Function { params, ret } => {
                params.iter().any(|p| p.contains_rc()) || ret.contains_rc()
            }
            _ => false,
        }
    }

    pub fn from_annotation(ann: &aelys_syntax::TypeAnnotation) -> Self {
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
            };
        }
        // must come before the uppercase guard, which would read it as Struct("Rc")
        if ann.name == "Rc" {
            let inner = ann
                .type_param
                .as_ref()
                .map(|p| Self::from_annotation(p))
                .unwrap_or(InferType::Dynamic);
            return InferType::Rc(Box::new(inner));
        }
        // Uppercase-starting names are always user-defined types (structs/enums).
        // Check this first so that names like "Void", "String", etc. are not
        // shadowed by the case-insensitive built-in type matching below.
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

    /// Returns `(bit_width, is_signed)` for numeric types, or just `None` for non-numeric
    /// floats use negative bit widths to separate them from integers in rank comp
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

    /// Returns `true` if `self` can be implicitly go to `target` without loss
    ///
    /// here's the rules
    ///
    /// - Signed -> wider signed: i8 -> i16, i8 -> i32, i8 -> i64, i16 -> i32, i16 -> i64, i32 -> i64
    /// - Unsigned -> wider unsigned: u8 -> u16, u8 -> u32, u8 -> u64, u16 -> u32, u16 -> u64, u32 -> u64
    /// - Unsigned -> wider signed (always fits): u8 -> i16, u8 - > i32, u8 -> i64, u16 -> i32, u16 -> i64, u32 -> i64
    /// - smoll int → float (exact): i8/u8/i16/u16 -> f32, any int <(or egal) 32 bits -> f64
    ///
    /// not allowed (because their lossy or they change sema)
    /// - signed to unsigned, unsigned to same-size signed, wider to narrower
    /// - i64/u64->f64, i32/u32->f32, float->int
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

        // le source must be an integer (not a float)
        if src_rank < 0 {
            return false;
        }

        // le target is a float
        if tgt_rank < 0 {
            let tgt_bits = -tgt_rank; // 32 or 64
            return if tgt_bits == 32 {
                // f32 has 24bit mantissa, exact for <(or egal) 16bit int
                src_rank <= 16
            } else {
                // f64 has 53bit mantissa, also exact for <(or equal) 32bit int
                src_rank <= 32
            };
        }

        // if they both are integers
        if src_signed && tgt_signed {
            // signed -> wider signed
            tgt_rank > src_rank
        } else if !src_signed && !tgt_signed {
            // unsigned -> wider unsigned
            tgt_rank > src_rank
        } else if !src_signed && tgt_signed {
            // unsigned -> wider signed (need strictly more bits to fit all values)
            tgt_rank > src_rank
        } else {
            // signed -> unsigned: nuh huh.
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
            InferType::Array(inner, Some(n)) => write!(f, "[{}; {}]", inner, n),
            InferType::Array(inner, None) => write!(f, "[{}]", inner),
            InferType::Vec(inner) => write!(f, "vec[{}]", inner),
            InferType::Rc(inner) => write!(f, "Rc<{}>", inner),
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
            InferType::Enum(name, type_args) => {
                write!(f, "{}", name)?;
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
