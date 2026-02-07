use crate::value::Value;

/// Type tag for array element specialization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TypeTag {
    Int = 0,
    Float = 1,
    Bool = 2,
    Object = 3,
}

impl TypeTag {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Int),
            1 => Some(Self::Float),
            2 => Some(Self::Bool),
            3 => Some(Self::Object),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ArrayData {
    Ints(Box<[i64]>),
    Floats(Box<[f64]>),
    Bools(Box<[u8]>),
    Objects(Box<[Value]>),
}

impl ArrayData {
    pub fn len(&self) -> usize {
        match self {
            Self::Ints(b) => b.len(),
            Self::Floats(b) => b.len(),
            Self::Bools(b) => b.len(),
            Self::Objects(b) => b.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn type_tag(&self) -> TypeTag {
        match self {
            Self::Ints(_) => TypeTag::Int,
            Self::Floats(_) => TypeTag::Float,
            Self::Bools(_) => TypeTag::Bool,
            Self::Objects(_) => TypeTag::Object,
        }
    }

    pub fn as_ints(&self) -> Option<&[i64]> {
        match self {
            Self::Ints(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_ints_mut(&mut self) -> Option<&mut [i64]> {
        match self {
            Self::Ints(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_floats(&self) -> Option<&[f64]> {
        match self {
            Self::Floats(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_floats_mut(&mut self) -> Option<&mut [f64]> {
        match self {
            Self::Floats(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_bools(&self) -> Option<&[u8]> {
        match self {
            Self::Bools(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_bools_mut(&mut self) -> Option<&mut [u8]> {
        match self {
            Self::Bools(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_objects(&self) -> Option<&[Value]> {
        match self {
            Self::Objects(b) => Some(b),
            _ => None,
        }
    }
    pub fn as_objects_mut(&mut self) -> Option<&mut [Value]> {
        match self {
            Self::Objects(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AelysArray {
    pub data: ArrayData,
}

impl AelysArray {
    pub fn new_ints(len: usize) -> Self {
        Self {
            data: ArrayData::Ints(vec![0i64; len].into_boxed_slice()),
        }
    }
    pub fn new_floats(len: usize) -> Self {
        Self {
            data: ArrayData::Floats(vec![0.0f64; len].into_boxed_slice()),
        }
    }
    pub fn new_bools(len: usize) -> Self {
        Self {
            data: ArrayData::Bools(vec![0u8; len].into_boxed_slice()),
        }
    }
    pub fn new_objects(len: usize) -> Self {
        Self {
            data: ArrayData::Objects(vec![Value::null(); len].into_boxed_slice()),
        }
    }

    pub fn from_ints(data: Vec<i64>) -> Self {
        Self {
            data: ArrayData::Ints(data.into_boxed_slice()),
        }
    }
    pub fn from_floats(data: Vec<f64>) -> Self {
        Self {
            data: ArrayData::Floats(data.into_boxed_slice()),
        }
    }
    pub fn from_bools(data: Vec<bool>) -> Self {
        Self {
            data: ArrayData::Bools(
                data.iter()
                    .map(|&b| b as u8)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
        }
    }
    pub fn from_objects(data: Vec<Value>) -> Self {
        Self {
            data: ArrayData::Objects(data.into_boxed_slice()),
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn type_tag(&self) -> TypeTag {
        self.data.type_tag()
    }

    pub fn get(&self, index: usize) -> Option<Value> {
        if index >= self.len() {
            return None;
        }
        Some(match &self.data {
            ArrayData::Ints(b) => Value::int(b[index]),
            ArrayData::Floats(b) => Value::float(b[index]),
            ArrayData::Bools(b) => Value::bool(b[index] != 0),
            ArrayData::Objects(b) => b[index],
        })
    }

    pub fn set(&mut self, index: usize, value: Value) -> bool {
        if index >= self.len() {
            return false;
        }
        match &mut self.data {
            ArrayData::Ints(b) => {
                if let Some(v) = value.as_int() {
                    b[index] = v;
                    true
                } else {
                    false
                }
            }
            ArrayData::Floats(b) => {
                if let Some(v) = value.as_float() {
                    b[index] = v;
                    true
                } else {
                    false
                }
            }
            ArrayData::Bools(b) => {
                if let Some(v) = value.as_bool() {
                    b[index] = v as u8;
                    true
                } else {
                    false
                }
            }
            ArrayData::Objects(b) => {
                b[index] = value;
                true
            }
        }
    }

    pub fn size_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.data {
                ArrayData::Ints(b) => b.len() * 8,
                ArrayData::Floats(b) => b.len() * 8,
                ArrayData::Bools(b) => b.len(),
                ArrayData::Objects(b) => b.len() * 8,
            }
    }
}
