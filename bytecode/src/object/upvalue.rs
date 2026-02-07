use crate::value::Value;

/// An upvalue captures a variable from an enclosing scope.
#[derive(Debug, Clone)]
pub struct AelysUpvalue {
    pub location: UpvalueLocation,
}

/// Where an upvalue's value is stored.
#[derive(Debug, Clone)]
pub enum UpvalueLocation {
    Open { frame_base: usize, register: u8 },
    Closed(Value),
}

impl AelysUpvalue {
    pub fn new_open(frame_base: usize, register: u8) -> Self {
        Self {
            location: UpvalueLocation::Open {
                frame_base,
                register,
            },
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.location, UpvalueLocation::Open { .. })
    }

    pub fn close(&mut self, value: Value) {
        self.location = UpvalueLocation::Closed(value);
    }
}
