//! Controlled document rendering core. The Python API is only an adapter.
pub mod css;
pub mod font;
pub mod html;
pub mod layout;
pub mod pdf;

#[cfg(feature = "python")]
mod python;

/// A diagnostic emitted for an unsupported or invalid construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
}

/// Errors from parsing, layout, resource loading, or PDF export.
#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
