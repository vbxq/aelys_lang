use std::fmt;

/// Reason for a constraint (for error messages)
#[derive(Debug, Clone)]
pub enum ConstraintReason {
    /// Binary operation requires compatible types
    BinaryOp { op: String },
    /// Bitwise operation requires integer types (FATAL error)
    BitwiseOp { op: String },
    /// Function call argument type
    Argument { func_name: String, arg_index: usize },
    /// Function return type
    Return { func_name: String },
    /// Variable assignment (reassignment)
    Assignment { var_name: String },
    /// Explicit type annotation on variable declaration (FATAL error)
    TypeAnnotation { var_name: String },
    /// If condition must be bool
    IfCondition,
    /// If branches must have same type
    IfBranches,
    /// While condition must be bool
    WhileCondition,
    /// For loop bounds must be int
    ForBounds,
    /// Comparison operands
    Comparison,
    /// Array element types must be consistent
    ArrayElement,
    /// Array index must be int
    ArrayIndex,
    /// Range bounds must be int
    RangeBound,
    /// Invalid cast (FATAL error)
    InvalidCast,
    /// Unknown type in annotation (fatal error)
    UnknownType { name: String },
    /// Generic constraint
    Other(String),
}

impl fmt::Display for ConstraintReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstraintReason::BinaryOp { op } => write!(f, "binary operator '{}'", op),
            ConstraintReason::BitwiseOp { op } => {
                write!(f, "bitwise operator '{}' (requires integers)", op)
            }
            ConstraintReason::Argument {
                func_name,
                arg_index,
            } => {
                write!(f, "argument {} to function '{}'", arg_index + 1, func_name)
            }
            ConstraintReason::Return { func_name } => {
                write!(f, "return type of function '{}'", func_name)
            }
            ConstraintReason::Assignment { var_name } => {
                write!(f, "assignment to variable '{}'", var_name)
            }
            ConstraintReason::TypeAnnotation { var_name } => {
                write!(f, "type annotation on variable '{}'", var_name)
            }
            ConstraintReason::IfCondition => write!(f, "if condition"),
            ConstraintReason::IfBranches => write!(f, "if/else branches"),
            ConstraintReason::WhileCondition => write!(f, "while condition"),
            ConstraintReason::ForBounds => write!(f, "for loop bounds"),
            ConstraintReason::Comparison => write!(f, "comparison"),
            ConstraintReason::ArrayElement => write!(f, "array element"),
            ConstraintReason::ArrayIndex => write!(f, "array index"),
            ConstraintReason::RangeBound => write!(f, "range bound"),
            ConstraintReason::InvalidCast => write!(f, "invalid cast"),
            ConstraintReason::UnknownType { name } => write!(f, "unknown type '{}'", name),
            ConstraintReason::Other(s) => write!(f, "{}", s),
        }
    }
}
