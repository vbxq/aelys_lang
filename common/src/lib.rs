pub mod diagnostic;
pub mod error;
pub mod result;
pub mod warning;

pub use diagnostic::color::ColorConfig;
pub use diagnostic::registry;
pub use diagnostic::render::render_summary;
pub use diagnostic::{Diagnostic, Label, Replacement, Severity, Suggestion};
pub use error::{AelysError, CompileError, CompileErrorKind};
pub use result::Result;
pub use warning::{Warning, WarningCollector, WarningConfig, WarningKind, format_warnings};
