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
    // a validated unicode scalar value, 4 bytes wide and never an integer
    Char,
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

    Struct(std::string::String, Vec<InferType>),
    Enum(std::string::String, Vec<InferType>),
    Dyn(std::string::String),

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
            InferType::Enum(_, args) | InferType::Struct(_, args) => {
                args.iter().any(|a| a.has_vars())
            }
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
            | InferType::Char
            | InferType::String
            | InferType::Null
            | InferType::Dyn(_) => true,
            InferType::Enum(_, args) | InferType::Struct(_, args) => {
                args.iter().all(|a| a.is_concrete())
            }
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
            InferType::Enum(_, args) | InferType::Struct(_, args) => {
                args.iter().any(|a| a.contains_rc())
            }
            InferType::Function { params, ret, .. } => {
                params.iter().any(|p| p.contains_rc()) || ret.contains_rc()
            }
            _ => false,
        }
    }

    pub fn subst_params(&self, params: &[String], args: &[InferType]) -> InferType {
        let go = |t: &InferType| t.subst_params(params, args);
        match self {
            InferType::Struct(name, a) if a.is_empty() => params
                .iter()
                .position(|p| p == name)
                .and_then(|i| args.get(i).cloned())
                .unwrap_or_else(|| self.clone()),
            InferType::Struct(name, a) => {
                InferType::Struct(name.clone(), a.iter().map(go).collect())
            }
            InferType::Enum(name, a) => InferType::Enum(name.clone(), a.iter().map(go).collect()),
            InferType::Function {
                params: ps,
                ret,
                nogc,
            } => InferType::Function {
                params: ps.iter().map(go).collect(),
                ret: Box::new(go(ret)),
                nogc: *nogc,
            },
            InferType::Array(inner, n) => InferType::Array(Box::new(go(inner)), *n),
            InferType::Vec(inner) => InferType::Vec(Box::new(go(inner))),
            InferType::Rc(inner) => InferType::Rc(Box::new(go(inner))),
            InferType::Ref { referent, mutable } => InferType::Ref {
                referent: Box::new(go(referent)),
                mutable: *mutable,
            },
            InferType::Slice { elem, mutable } => InferType::Slice {
                elem: Box::new(go(elem)),
                mutable: *mutable,
            },
            InferType::Tuple(elems) => InferType::Tuple(elems.iter().map(go).collect()),
            other => other.clone(),
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
        if ann.is_dyn {
            return InferType::Dyn(ann.name.clone());
        }
        if let Some(elems) = &ann.tuple_elems {
            return InferType::Tuple(elems.iter().map(Self::from_annotation).collect());
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
            return InferType::Struct(ann.name.clone(), Vec::new());
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
            "char" => InferType::Char,
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
            "char" => InferType::Char,
            "string" | "str" => InferType::String,
            "null" | "void" => InferType::Null,
            _ => {
                if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                    InferType::Struct(name.to_string(), Vec::new())
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

    /// the one list `<` accepts, so the `ord` bound cannot drift away from the operator
    pub fn ord_types() -> Vec<InferType> {
        let mut types = Self::all_numeric_types();
        types.push(InferType::Char);
        types.push(InferType::String);
        types
    }

    pub fn eq_types() -> Vec<InferType> {
        let mut types = Self::ord_types();
        types.push(InferType::Bool);
        types
    }

    pub fn satisfies_ord(&self) -> bool {
        Self::ord_types().contains(self)
    }

    pub fn int_bounds(&self) -> Option<(i128, i128)> {
        match self {
            InferType::I8 => Some((i8::MIN as i128, i8::MAX as i128)),
            InferType::I16 => Some((i16::MIN as i128, i16::MAX as i128)),
            InferType::I32 => Some((i32::MIN as i128, i32::MAX as i128)),
            InferType::I64 => Some((i64::MIN as i128, i64::MAX as i128)),
            InferType::U8 => Some((0, u8::MAX as i128)),
            InferType::U16 => Some((0, u16::MAX as i128)),
            InferType::U32 => Some((0, u32::MAX as i128)),
            InferType::U64 => Some((0, u64::MAX as i128)),
            _ => None,
        }
    }

    pub fn satisfies_eq(&self) -> bool {
        match self {
            InferType::Tuple(elems) => elems.iter().all(InferType::satisfies_eq),
            _ => Self::eq_types().contains(self),
        }
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

pub struct QualifiedType<'a>(&'a InferType);

impl fmt::Display for QualifiedType<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.render(f, true)
    }
}

impl InferType {
    pub fn qualified(&self) -> QualifiedType<'_> {
        QualifiedType(self)
    }

    pub fn shown_pair(a: &InferType, b: &InferType) -> (String, String) {
        let (left, right) = (a.to_string(), b.to_string());
        if left == right && a != b {
            (a.qualified().to_string(), b.qualified().to_string())
        } else {
            (left, right)
        }
    }

    fn render(&self, f: &mut fmt::Formatter<'_>, qualified: bool) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, items: &[InferType]| -> fmt::Result {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                item.render(f, qualified)?;
            }
            Ok(())
        };
        let name = |n: &str| -> String {
            if qualified {
                crate::modules::strip_type_head(n).to_string()
            } else {
                crate::modules::source_type_name(n).to_string()
            }
        };
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
            InferType::Char => write!(f, "char"),
            InferType::String => write!(f, "string"),
            InferType::Null => write!(f, "()"),
            InferType::Never => write!(f, "Never"),
            InferType::Function { params, ret, nogc } => {
                write!(f, "{}fn(", if *nogc { "nogc " } else { "" })?;
                list(f, params)?;
                write!(f, ")")?;
                if !matches!(**ret, InferType::Null) {
                    write!(f, " -> ")?;
                    ret.render(f, qualified)?;
                }
                Ok(())
            }
            InferType::Array(inner, size) => {
                write!(f, "[")?;
                inner.render(f, qualified)?;
                match size {
                    Some(n) => write!(f, "; {}]", n),
                    None => write!(f, "]"),
                }
            }
            InferType::Vec(inner) => {
                write!(f, "Vec<")?;
                inner.render(f, qualified)?;
                write!(f, ">")
            }
            InferType::Rc(inner) => {
                write!(f, "Rc<")?;
                inner.render(f, qualified)?;
                write!(f, ">")
            }
            InferType::Ref { referent, mutable } => {
                write!(f, "&{}", if *mutable { "mut " } else { "" })?;
                referent.render(f, qualified)
            }
            InferType::Slice { elem, mutable } => {
                write!(f, "&{}[", if *mutable { "mut " } else { "" })?;
                elem.render(f, qualified)?;
                write!(f, "]")
            }
            InferType::Tuple(elems) => {
                write!(f, "(")?;
                list(f, elems)?;
                write!(f, ")")
            }
            InferType::Range => write!(f, "range"),
            InferType::Struct(n, type_args) | InferType::Enum(n, type_args) => {
                write!(f, "{}", name(n))?;
                if !type_args.is_empty() {
                    write!(f, "<")?;
                    list(f, type_args)?;
                    write!(f, ">")?;
                }
                Ok(())
            }
            InferType::Dyn(q) => write!(f, "dyn {}", name(q)),
            InferType::Var(_) | InferType::Dynamic => write!(f, "_"),
        }
    }
}

impl fmt::Display for InferType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.render(f, false)
    }
}
