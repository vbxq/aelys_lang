use super::Value;

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_null() {
            write!(f, "null")
        } else if let Some(b) = self.as_bool() {
            write!(f, "{}", b)
        } else if let Some(n) = self.as_int() {
            write!(f, "{}", n)
        } else if let Some(n) = self.as_float() {
            if n.fract() == 0.0 {
                write!(f, "{}.0", n)
            } else {
                write!(f, "{}", n)
            }
        } else if let Some(p) = self.as_ptr() {
            write!(f, "<ptr:{}>", p)
        } else {
            write!(f, "<unknown:{:#x}>", self.0)
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_null() {
            write!(f, "null")
        } else if let Some(b) = self.as_bool() {
            write!(f, "{}", b)
        } else if let Some(n) = self.as_int() {
            write!(f, "{}", n)
        } else if let Some(n) = self.as_float() {
            if n.fract() == 0.0 {
                write!(f, "{}.0", n)
            } else {
                write!(f, "{}", n)
            }
        } else if let Some(_p) = self.as_ptr() {
            write!(f, "<object>")
        } else {
            write!(f, "<unknown>")
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::null()
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        if self.0 == other.0 {
            return true;
        }
        if self.is_float() && other.is_float() {
            return self.as_float() == other.as_float();
        }
        if let (Some(a), Some(b)) = (self.as_int(), other.as_float()) {
            return (a as f64) == b;
        }
        if let (Some(a), Some(b)) = (self.as_float(), other.as_int()) {
            return a == (b as f64);
        }
        false
    }
}
