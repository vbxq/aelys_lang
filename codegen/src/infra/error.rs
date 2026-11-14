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
    // the host toolchain refused: no target for the triple, no machine, or no write
    Toolchain(String),
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
            Self::LlvmError(message) | Self::Toolchain(message) => {
                write!(f, "llvm error: {message}")
            }
            Self::UnsupportedType(message) => write!(f, "unsupported type: {message}"),
            Self::UnsupportedInstruction(message) => {
                write!(f, "unsupported instruction: {message}")
            }
            Self::InvalidNativeEntry(message) => write!(f, "invalid native entry: {message}"),
            Self::UnsupportedAir {
                kind,
                detail,
                location,
            } => {
                write!(f, "unsupported AIR: {kind}")?;
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
