use super::ObjectKind;

#[derive(Debug)]
pub struct GcObject {
    pub marked: bool, // for mark-sweep
    pub kind: ObjectKind,
}

impl GcObject {
    pub fn new(kind: ObjectKind) -> Self {
        Self {
            marked: false,
            kind,
        }
    }
}
