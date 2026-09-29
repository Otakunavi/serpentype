//! PyO3 adapter. Expensive layout and export release the Python GIL.
use crate::font::FontRegistry;
use crate::layout::{PreparedDocument, Renderer};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::path::PathBuf;
use std::sync::Arc;

fn pyerr(error: crate::Error) -> PyErr {
    PyValueError::new_err(error.to_string())
}

#[pyclass(name = "Diagnostic", frozen)]
#[derive(Clone)]
pub struct PyDiagnostic {
    #[pyo3(get)]
    pub code: String,
    #[pyo3(get)]
    pub message: String,
}
#[pyclass(name = "CacheStats", frozen)]
pub struct PyCacheStats {
    #[pyo3(get)]
    pub bytes: usize,
    #[pyo3(get)]
    pub entries: usize,
    #[pyo3(get)]
    pub hits: usize,
    #[pyo3(get)]
    pub misses: usize,
}

/// Explicit font registrations shared by renderers and documents.
#[pyclass(name = "FontRegistry")]
pub struct PyFontRegistry {
    inner: FontRegistry,
}
#[pymethods]
impl PyFontRegistry {
    #[new]
    #[pyo3(signature=(cache_bytes=67108864))]
    fn new(cache_bytes: usize) -> Self {
        Self {
            inner: FontRegistry::new(cache_bytes),
        }
    }
    #[pyo3(signature=(path, family, weight=400, style="normal"))]
    fn register_file(&self, path: &str, family: &str, weight: u16, style: &str) -> PyResult<()> {
        self.inner
            .register_file(path, family, weight, style)
            .map_err(pyerr)
    }
    #[pyo3(signature=(data, family, weight=400, style="normal"))]
    fn register_bytes(&self, data: &[u8], family: &str, weight: u16, style: &str) -> PyResult<()> {
        self.inner
            .register_bytes(data.to_vec(), family, weight, style)
            .map_err(pyerr)
    }
    fn clear_cache(&self) {
        self.inner.clear_cache()
    }
    fn cache_stats(&self) -> PyCacheStats {
        let s = self.inner.stats();
        PyCacheStats {
            bytes: s.bytes,
            entries: s.entries,
            hits: s.hits,
            misses: s.misses,
        }
    }
}

/// Stateless call surface; font cache locks are held only for short lookups.
#[pyclass(name = "Renderer", frozen)]
pub struct PyRenderer {
    inner: Renderer,
}
#[pymethods]
impl PyRenderer {
    #[new]
    #[pyo3(signature=(fonts=None, base_dir=".", strict=false))]
    fn new(fonts: Option<PyRef<'_, PyFontRegistry>>, base_dir: &str, strict: bool) -> Self {
        let registry = fonts
            .map(|f| f.inner.clone())
            .unwrap_or_else(|| FontRegistry::new(64 * 1024 * 1024));
        Self {
            inner: Renderer::new(registry, PathBuf::from(base_dir), strict),
        }
    }
    #[pyo3(signature=(html, css=""))]
    fn layout(&self, py: Python<'_>, html: &str, css: &str) -> PyResult<PyPreparedDocument> {
        let renderer = self.inner.clone();
        let html = html.to_owned();
        let css = css.to_owned();
        let doc = py
            .allow_threads(move || renderer.layout(&html, &css))
            .map_err(pyerr)?;
        Ok(PyPreparedDocument {
            inner: Arc::new(doc),
        })
    }
}

/// Immutable prepared pages and resources, reusable for multiple PDF exports.
#[pyclass(name = "PreparedDocument", frozen)]
pub struct PyPreparedDocument {
    inner: Arc<PreparedDocument>,
}
#[pymethods]
impl PyPreparedDocument {
    #[getter]
    fn page_count(&self) -> usize {
        self.inner.page_count()
    }
    #[getter]
    fn warnings(&self) -> Vec<PyDiagnostic> {
        self.inner
            .warnings
            .iter()
            .map(|w| PyDiagnostic {
                code: w.code.into(),
                message: w.message.clone(),
            })
            .collect()
    }
    fn to_pdf<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let doc = self.inner.clone();
        let bytes = py.allow_threads(move || doc.to_pdf()).map_err(pyerr)?;
        Ok(PyBytes::new(py, &bytes))
    }
}

#[pymodule]
fn _serpentype(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDiagnostic>()?;
    m.add_class::<PyCacheStats>()?;
    m.add_class::<PyFontRegistry>()?;
    m.add_class::<PyRenderer>()?;
    m.add_class::<PyPreparedDocument>()?;
    Ok(())
}
