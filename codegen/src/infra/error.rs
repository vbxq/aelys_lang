// TODO: rewrite this entire thing and use our common module, this is temporary until we get rid of the VM

use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirNodePosition {
    Stmt(usize),
    Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AirNodeLocation {
    pub function: String,
    pub block: Option<u32>,
    pub position: AirNodePosition,
}

#[derive(Debug)]
pub enum LlvmBackendError {
    LlvmError(String),
    UnsupportedType(String),
    UnsupportedInstruction(String),
    InvalidNativeEntry(String),
    UnsupportedAir {
        kind: &'static str,
        detail: String,
        location: Option<AirNodeLocation>,
    },
}

impl LlvmBackendError {
    pub fn unsupported(kind: &'static str, detail: impl Into<String>) -> Self {
        Self::UnsupportedAir {
            kind,
            detail: detail.into(),
            location: None,
        }
    }

    pub fn unsupported_with_location(
        kind: &'static str,
        detail: impl Into<String>,
        location: AirNodeLocation,
    ) -> Self {
        Self::UnsupportedAir {
            kind,
            detail: detail.into(),
            location: Some(location),
        }
    }
}

impl Display for LlvmBackendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LlvmError(message) => write!(f, "error[llvm-backend]: llvm error: {message}"),
            Self::UnsupportedType(message) => {
                write!(f, "error[llvm-backend]: unsupported type: {message}")
            }
            Self::UnsupportedInstruction(message) => {
                write!(f, "error[llvm-backend]: unsupported instruction: {message}")
            }
            Self::InvalidNativeEntry(message) => {
                write!(f, "error[llvm-backend]: invalid native entry: {message}")
            }
            Self::UnsupportedAir {
                kind,
                detail,
                location,
            } => {
                write!(f, "error[llvm-backend]: unsupported AIR: {kind}")?;
                if !detail.is_empty() {
                    write!(f, " (reason: {detail})")?;
                }
                if let Some(location) = location {
                    write!(f, " at fn `{}`", location.function)?;
                    if let Some(block) = location.block {
                        write!(f, ", bb{block}")?;
                    }
                    match location.position {
                        AirNodePosition::Stmt(index) => write!(f, ", stmt #{index}")?,
                        AirNodePosition::Terminator => write!(f, ", terminator")?,
                    }
                }
                Ok(())
            }
        }
    }
}

impl Error for LlvmBackendError {}
