use super::array::{AelysArray, ArrayData, TypeTag};
use crate::value::Value;

#[derive(Debug, Clone)]
pub enum VecData {
    Ints(Vec<i64>),
    Floats(Vec<f64>),
    Bools(Vec<u8>),
    Objects(Vec<Value>),
}

impl VecData {
    pub fn len(&self) -> usize {
        match self {
            Self::Ints(v) => v.len(),
            Self::Floats(v) => v.len(),
            Self::Bools(v) => v.len(),
            Self::Objects(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        match self {
            Self::Ints(v) => v.capacity(),
            Self::Floats(v) => v.capacity(),
            Self::Bools(v) => v.capacity(),
            Self::Objects(v) => v.capacity(),
        }
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
            Self::Ints(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_ints_mut(&mut self) -> Option<&mut Vec<i64>> {
        match self {
            Self::Ints(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_floats(&self) -> Option<&[f64]> {
        match self {
            Self::Floats(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_floats_mut(&mut self) -> Option<&mut Vec<f64>> {
        match self {
            Self::Floats(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_bools(&self) -> Option<&[u8]> {
        match self {
            Self::Bools(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_bools_mut(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            Self::Bools(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_objects(&self) -> Option<&[Value]> {
        match self {
            Self::Objects(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_objects_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Self::Objects(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AelysVec {
    pub data: VecData,
}

impl AelysVec {
    pub fn new_ints() -> Self {
        Self {
            data: VecData::Ints(Vec::new()),
        }
    }
    pub fn new_floats() -> Self {
        Self {
            data: VecData::Floats(Vec::new()),
        }
    }
    pub fn new_bools() -> Self {
        Self {
            data: VecData::Bools(Vec::new()),
        }
    }
    pub fn new_objects() -> Self {
        Self {
            data: VecData::Objects(Vec::new()),
        }
    }

    pub fn with_capacity_ints(cap: usize) -> Self {
        Self {
            data: VecData::Ints(Vec::with_capacity(cap)),
        }
    }
    pub fn with_capacity_floats(cap: usize) -> Self {
        Self {
            data: VecData::Floats(Vec::with_capacity(cap)),
        }
    }
    pub fn with_capacity_bools(cap: usize) -> Self {
        Self {
            data: VecData::Bools(Vec::with_capacity(cap)),
        }
    }
    pub fn with_capacity_objects(cap: usize) -> Self {
        Self {
            data: VecData::Objects(Vec::with_capacity(cap)),
        }
    }

    pub fn from_ints(data: Vec<i64>) -> Self {
        Self {
            data: VecData::Ints(data),
        }
    }
    pub fn from_floats(data: Vec<f64>) -> Self {
        Self {
            data: VecData::Floats(data),
        }
    }
    pub fn from_bools(data: Vec<bool>) -> Self {
        Self {
            data: VecData::Bools(data.iter().map(|&b| b as u8).collect()),
        }
    }
    pub fn from_objects(data: Vec<Value>) -> Self {
        Self {
            data: VecData::Objects(data),
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }
    pub fn type_tag(&self) -> TypeTag {
        self.data.type_tag()
    }

    pub fn get(&self, index: usize) -> Option<Value> {
        if index >= self.len() {
            return None;
        }
        Some(match &self.data {
            VecData::Ints(v) => Value::int(v[index]),
            VecData::Floats(v) => Value::float(v[index]),
            VecData::Bools(v) => Value::bool(v[index] != 0),
            VecData::Objects(v) => v[index],
        })
    }

    pub fn set(&mut self, index: usize, value: Value) -> bool {
        if index >= self.len() {
            return false;
        }
        match &mut self.data {
            VecData::Ints(v) => {
                if let Some(val) = value.as_int() {
                    v[index] = val;
                    true
                } else {
                    false
                }
            }
            VecData::Floats(v) => {
                if let Some(val) = value.as_float() {
                    v[index] = val;
                    true
                } else {
                    false
                }
            }
            VecData::Bools(v) => {
                if let Some(val) = value.as_bool() {
                    v[index] = val as u8;
                    true
                } else {
                    false
                }
            }
            VecData::Objects(v) => {
                v[index] = value;
                true
            }
        }
    }

    pub fn push(&mut self, value: Value) -> bool {
        match &mut self.data {
            VecData::Ints(v) => {
                if let Some(val) = value.as_int() {
                    v.push(val);
                    true
                } else {
                    false
                }
            }
            VecData::Floats(v) => {
                if let Some(val) = value.as_float() {
                    v.push(val);
                    true
                } else {
                    false
                }
            }
            VecData::Bools(v) => {
                if let Some(val) = value.as_bool() {
                    v.push(val as u8);
                    true
                } else {
                    false
                }
            }
            VecData::Objects(v) => {
                v.push(value);
                true
            }
        }
    }

    pub fn pop(&mut self) -> Option<Value> {
        match &mut self.data {
            VecData::Ints(v) => v.pop().map(Value::int),
            VecData::Floats(v) => v.pop().map(Value::float),
            VecData::Bools(v) => v.pop().map(|b| Value::bool(b != 0)),
            VecData::Objects(v) => v.pop(),
        }
    }

    pub fn reserve(&mut self, additional: usize) {
        match &mut self.data {
            VecData::Ints(v) => v.reserve(additional),
            VecData::Floats(v) => v.reserve(additional),
            VecData::Bools(v) => v.reserve(additional),
            VecData::Objects(v) => v.reserve(additional),
        }
    }

    pub fn clear(&mut self) {
        match &mut self.data {
            VecData::Ints(v) => v.clear(),
            VecData::Floats(v) => v.clear(),
            VecData::Bools(v) => v.clear(),
            VecData::Objects(v) => v.clear(),
        }
    }

    pub fn shrink_to_fit(&mut self) {
        match &mut self.data {
            VecData::Ints(v) => v.shrink_to_fit(),
            VecData::Floats(v) => v.shrink_to_fit(),
            VecData::Bools(v) => v.shrink_to_fit(),
            VecData::Objects(v) => v.shrink_to_fit(),
        }
    }

    pub fn to_array(&self) -> AelysArray {
        AelysArray {
            data: match &self.data {
                VecData::Ints(v) => ArrayData::Ints(v.clone().into_boxed_slice()),
                VecData::Floats(v) => ArrayData::Floats(v.clone().into_boxed_slice()),
                VecData::Bools(v) => ArrayData::Bools(v.clone().into_boxed_slice()),
                VecData::Objects(v) => ArrayData::Objects(v.clone().into_boxed_slice()),
            },
        }
    }

    pub fn objects(&self) -> Option<&[Value]> {
        match &self.data {
            VecData::Objects(v) => Some(v),
            _ => None,
        }
    }

    pub fn size_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + match &self.data {
                VecData::Ints(v) => v.capacity() * 8,
                VecData::Floats(v) => v.capacity() * 8,
                VecData::Bools(v) => v.capacity(),
                VecData::Objects(v) => v.capacity() * 8,
            }
    }
}
