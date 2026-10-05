//! Controlled document rendering core. The Python API is only an adapter.
pub mod css;
pub mod font;
pub mod html;
pub mod layout;
pub mod pdf;

/// Catch internal unwind panics before they can cross a foreign-function boundary.
#[cfg(any(feature = "python", test))]
pub(crate) fn catch_internal<T>(operation: impl FnOnce() -> T) -> std::result::Result<T, ()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|_| ())
}

#[cfg(test)]
mod ffi_safety_tests {
    use super::catch_internal;

    #[test]
    fn panic_in_foreign_boundary_is_caught() {
        let result = catch_internal(|| panic!("injected renderer panic"));
        assert!(result.is_err());
    }

    #[test]
    fn ordinary_result_crosses_boundary_unchanged() {
        assert_eq!(catch_internal(|| 42), Ok(42));
    }
}

#[cfg(feature = "python")]
mod python;

/// A diagnostic emitted for an unsupported or invalid construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
    pub source: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub selector: Option<String>,
    pub property: Option<String>,
    pub value: Option<String>,
    pub occurrences: usize,
}
impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: "warning",
            code,
            message: message.into(),
            source: None,
            line: None,
            column: None,
            selector: None,
            property: None,
            value: None,
            occurrences: 1,
        }
    }
    pub fn property(mut self, property: &str, value: &str) -> Self {
        self.property = Some(property.into());
        self.value = Some(value.into());
        self
    }
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
