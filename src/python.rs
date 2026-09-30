//! PyO3 adapter. Expensive layout and export release the Python GIL.
use crate::font::FontRegistry;
use crate::layout::{PreparedDocument, RenderLimits, RenderStats, Renderer};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pyo3::create_exception!(_serpentype, RenderLimitError, PyValueError);
pyo3::create_exception!(_serpentype, RenderCancelledError, PyValueError);
pyo3::create_exception!(_serpentype, MissingGlyphError, PyValueError);

fn missing_glyph_error(message: &str) -> Option<PyErr> {
    let details = message.strip_prefix("no registered font for U+")?;
    let (glyph, request) = details.split_once("; requested ")?;
    let code_point = u32::from_str_radix(glyph.split_once(' ')?.0, 16).ok()?;
    let character = char::from_u32(code_point)?;
    let (request, location) = request
        .split_once("; location ")
        .map_or((request, None), |(request, location)| {
            (request, Some(location))
        });
    let (family, weight_style) = request.rsplit_once(" weight ")?;
    let (weight, style) = weight_style.split_once(' ')?;
    let weight = weight.parse::<u16>().ok()?;
    let error = MissingGlyphError::new_err(message.to_owned());
    Python::with_gil(|py| {
        let value = error.value(py);
        let _ = value.setattr("code_point", code_point);
        let _ = value.setattr("character", character.to_string());
        let _ = value.setattr("requested_family", family);
        let _ = value.setattr("weight", weight);
        let _ = value.setattr("style", style);
        let _ = value.setattr("selected_fallback", py.None());
        if let Some(location) = location {
            let mut parts = location.split_whitespace();
            let source = parts.next();
            let line = parts.next().and_then(|value| value.parse::<usize>().ok());
            let column = parts.next().and_then(|value| value.parse::<usize>().ok());
            let _ = value.setattr("source", source);
            let _ = value.setattr("line", line);
            let _ = value.setattr("column", column);
        } else {
            let _ = value.setattr("source", py.None());
            let _ = value.setattr("line", py.None());
            let _ = value.setattr("column", py.None());
        }
    });
    Some(error)
}

fn pyerr(error: crate::Error) -> PyErr {
    let message = error.to_string();
    if message.starts_with("render limit exceeded:") {
        RenderLimitError::new_err(message)
    } else if message == "render cancelled" {
        RenderCancelledError::new_err(message)
    } else if let Some(error) = missing_glyph_error(&message) {
        error
    } else {
        PyValueError::new_err(message)
    }
}

#[pyclass(name = "RenderLimits", frozen)]
pub struct PyRenderLimits {
    inner: RenderLimits,
}

#[pyclass(name = "RenderStats", frozen)]
pub struct PyRenderStats {
    #[pyo3(get)]
    parse_ms: f64,
    #[pyo3(get)]
    css_cascade_ms: f64,
    #[pyo3(get)]
    shaping_ms: f64,
    #[pyo3(get)]
    layout_ms: f64,
    #[pyo3(get)]
    pdf_export_ms: f64,
    #[pyo3(get)]
    page_count: usize,
    #[pyo3(get)]
    dom_node_count: usize,
    #[pyo3(get)]
    glyph_count: usize,
    #[pyo3(get)]
    font_cache_hits: usize,
    #[pyo3(get)]
    font_cache_misses: usize,
    #[pyo3(get)]
    image_cache_hits: usize,
    #[pyo3(get)]
    image_cache_misses: usize,
    #[pyo3(get)]
    retained_bytes: usize,
    #[pyo3(get)]
    output_size: usize,
    #[pyo3(get)]
    diagnostic_count: usize,
}
impl From<RenderStats> for PyRenderStats {
    fn from(s: RenderStats) -> Self {
        Self {
            parse_ms: s.parse_ms,
            css_cascade_ms: s.css_cascade_ms,
            shaping_ms: s.shaping_ms,
            layout_ms: s.layout_ms,
            pdf_export_ms: s.pdf_export_ms,
            page_count: s.page_count,
            dom_node_count: s.dom_node_count,
            glyph_count: s.glyph_count,
            font_cache_hits: s.font_cache_hits,
            font_cache_misses: s.font_cache_misses,
            image_cache_hits: s.image_cache_hits,
            image_cache_misses: s.image_cache_misses,
            retained_bytes: s.retained_bytes,
            output_size: s.output_size,
            diagnostic_count: s.diagnostic_count,
        }
    }
}
#[pymethods]
impl PyRenderLimits {
    #[new]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature=(max_pages=500, max_input_bytes=20_000_000, max_nodes=200_000,
        max_css_rules=50_000, max_image_pixels=50_000_000, max_resource_bytes=100_000_000,
        max_layout_iterations=1_000_000, restrict_base_dir=false))]
    fn new(
        max_pages: usize,
        max_input_bytes: usize,
        max_nodes: usize,
        max_css_rules: usize,
        max_image_pixels: u64,
        max_resource_bytes: usize,
        max_layout_iterations: usize,
        restrict_base_dir: bool,
    ) -> Self {
        Self {
            inner: RenderLimits {
                max_pages,
                max_input_bytes,
                max_nodes,
                max_css_rules,
                max_image_pixels,
                max_resource_bytes,
                max_layout_iterations,
                restrict_base_dir,
            },
        }
    }
    #[getter]
    fn max_pages(&self) -> usize {
        self.inner.max_pages
    }
    #[getter]
    fn max_input_bytes(&self) -> usize {
        self.inner.max_input_bytes
    }
    #[getter]
    fn max_nodes(&self) -> usize {
        self.inner.max_nodes
    }
    #[getter]
    fn max_css_rules(&self) -> usize {
        self.inner.max_css_rules
    }
    #[getter]
    fn max_image_pixels(&self) -> u64 {
        self.inner.max_image_pixels
    }
    #[getter]
    fn max_resource_bytes(&self) -> usize {
        self.inner.max_resource_bytes
    }
    #[getter]
    fn max_layout_iterations(&self) -> usize {
        self.inner.max_layout_iterations
    }
    #[getter]
    fn restrict_base_dir(&self) -> bool {
        self.inner.restrict_base_dir
    }
}

#[pyclass(name = "CancelToken")]
pub struct PyCancelToken {
    inner: Arc<AtomicBool>,
}
#[pymethods]
impl PyCancelToken {
    #[new]
    fn new() -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(false)),
        }
    }
    fn cancel(&self) {
        self.inner.store(true, Ordering::Relaxed);
    }
    #[getter]
    fn cancelled(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }
}

#[pyclass(name = "Diagnostic", frozen)]
#[derive(Clone)]
pub struct PyDiagnostic {
    #[pyo3(get)]
    pub severity: String,
    #[pyo3(get)]
    pub code: String,
    #[pyo3(get)]
    pub message: String,
    #[pyo3(get)]
    pub source: Option<String>,
    #[pyo3(get)]
    pub line: Option<usize>,
    #[pyo3(get)]
    pub column: Option<usize>,
    #[pyo3(get)]
    pub selector: Option<String>,
    #[pyo3(get)]
    pub property: Option<String>,
    #[pyo3(get)]
    pub value: Option<String>,
    #[pyo3(get)]
    pub occurrences: usize,
}
impl From<&crate::Diagnostic> for PyDiagnostic {
    fn from(d: &crate::Diagnostic) -> Self {
        Self {
            severity: d.severity.into(),
            code: d.code.into(),
            message: d.message.clone(),
            source: d.source.clone(),
            line: d.line,
            column: d.column,
            selector: d.selector.clone(),
            property: d.property.clone(),
            value: d.value.clone(),
            occurrences: d.occurrences,
        }
    }
}
#[pymethods]
impl PyDiagnostic {
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("severity", &self.severity)?;
        d.set_item("code", &self.code)?;
        d.set_item("message", &self.message)?;
        d.set_item("source", &self.source)?;
        d.set_item("line", self.line)?;
        d.set_item("column", self.column)?;
        d.set_item("selector", &self.selector)?;
        d.set_item("property", &self.property)?;
        d.set_item("value", &self.value)?;
        d.set_item("occurrences", self.occurrences)?;
        Ok(d)
    }
    fn to_json(&self, py: Python<'_>) -> PyResult<String> {
        py.import("json")?
            .call_method1("dumps", (self.to_dict(py)?,))?
            .extract()
    }
}

const CAPABILITIES: &[(&str, &str)] = &[
    ("pdf.deterministic", "partial"),
    ("css.text-align.justify", "partial"),
    ("css.text-indent", "partial"),
    ("css.length.em", "partial"),
    ("css.length.rem", "partial"),
    ("css.length.calc", "partial"),
    ("css.box-sizing", "partial"),
    ("css.box.min-max", "partial"),
    ("css.visibility", "partial"),
    ("css.color.named", "full"),
    ("css.color.rgb", "partial"),
    ("css.color.hsl", "partial"),
    ("css.color.alpha", "unsupported"),
    ("font.shaping", "experimental"),
    ("font.bidi.mixed", "experimental"),
    ("font.bidi.controls", "experimental"),
    ("font.fallback", "partial"),
    ("font.fallback.cluster", "partial"),
    ("font.fallback.emoji-mono", "partial"),
    ("font.cff", "partial"),
    ("font.woff", "partial"),
    ("font.woff2", "partial"),
    ("font.variable-weight", "partial"),
    ("font.variable-width", "partial"),
    ("font.synthetic-bold", "partial"),
    ("font.synthetic-italic", "partial"),
    ("font.missing-glyph-diagnostic", "partial"),
    ("html.table.rowspan", "partial"),
    ("html.table.colspan", "partial"),
    ("html.table.repeat-tfoot", "partial"),
    ("image.svg", "partial"),
    ("image.data-uri", "partial"),
    ("pdf.links", "partial"),
    ("pdf.metadata", "partial"),
    ("resource.loader", "partial"),
    ("resource.local-base-dir-confinement", "full"),
    ("render.limits", "partial"),
    ("render.cancellation", "partial"),
    ("html.inline.basic", "partial"),
    ("css.paged.named-pages", "partial"),
    ("css.paged.recto-verso", "partial"),
    ("css.paged.pseudo-pages", "partial"),
    ("css.position.relative", "partial"),
    ("image.png", "partial"),
    ("image.jpeg", "partial"),
    ("image.webp", "partial"),
    ("image.gif", "partial"),
    ("layout.flex", "partial"),
    ("layout.grid", "partial"),
    ("layout.gap", "partial"),
];
#[pyfunction]
fn capabilities() -> std::collections::BTreeMap<&'static str, &'static str> {
    CAPABILITIES.iter().copied().collect()
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
    fn supports(&self, feature: &str) -> bool {
        if matches!(
            feature,
            "font.shaping" | "font.bidi.mixed" | "font.bidi.controls"
        ) {
            return self.inner.experimental_shaping;
        }
        CAPABILITIES
            .iter()
            .any(|(name, level)| *name == feature && matches!(*level, "full" | "partial"))
    }
    #[new]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature=(fonts=None, base_dir=".", strict=false, limits=None, experimental_shaping=false, svg_dpi=144.0, synthetic_bold=false, synthetic_italic=false))]
    fn new(
        fonts: Option<PyRef<'_, PyFontRegistry>>,
        base_dir: &str,
        strict: bool,
        limits: Option<PyRef<'_, PyRenderLimits>>,
        experimental_shaping: bool,
        svg_dpi: f32,
        synthetic_bold: bool,
        synthetic_italic: bool,
    ) -> PyResult<Self> {
        if !svg_dpi.is_finite() || svg_dpi <= 0.0 {
            return Err(PyValueError::new_err(
                "svg_dpi must be a positive finite number",
            ));
        }
        let registry = fonts
            .map(|f| f.inner.clone())
            .unwrap_or_else(|| FontRegistry::new(64 * 1024 * 1024));
        let mut inner = Renderer::new(registry, PathBuf::from(base_dir), strict);
        if let Some(limits) = limits {
            inner.limits = limits.inner.clone();
        }
        inner.experimental_shaping = experimental_shaping;
        inner.svg_dpi = svg_dpi;
        inner.synthetic_bold = synthetic_bold;
        inner.synthetic_italic = synthetic_italic;
        Ok(Self { inner })
    }
    #[pyo3(signature=(html, css="", cancel_token=None))]
    fn layout(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        cancel_token: Option<PyRef<'_, PyCancelToken>>,
    ) -> PyResult<PyPreparedDocument> {
        let renderer = self.inner.clone();
        let html = html.to_owned();
        let css = css.to_owned();
        let token = cancel_token.map(|t| t.inner.clone());
        let result = py.allow_threads(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                renderer.layout_with_cancel(&html, &css, token)
            }))
        });
        let doc = result
            .map_err(|_| PyRuntimeError::new_err("renderer failed internally"))?
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
    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("title", &self.inner.metadata.title)?;
        d.set_item("author", &self.inner.metadata.author)?;
        d.set_item("subject", &self.inner.metadata.subject)?;
        d.set_item("keywords", &self.inner.metadata.keywords)?;
        d.set_item("language", &self.inner.metadata.language)?;
        Ok(d)
    }
    #[getter]
    fn render_stats(&self) -> PyResult<PyRenderStats> {
        self.inner
            .stats
            .lock()
            .map(|s| s.clone().into())
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }
    #[getter]
    fn overflow_count(&self) -> usize {
        0
    }
    #[getter]
    fn missing_glyphs(&self) -> Vec<String> {
        vec![]
    }
    #[getter]
    fn unsupported_features(&self) -> Vec<String> {
        self.inner
            .warnings
            .iter()
            .filter(|d| d.message.contains("unsupported"))
            .map(|d| d.message.clone())
            .collect()
    }
    #[getter]
    fn page_count(&self) -> usize {
        self.inner.page_count()
    }
    #[getter]
    fn warnings(&self) -> Vec<PyDiagnostic> {
        self.inner.warnings.iter().map(PyDiagnostic::from).collect()
    }
    #[getter]
    fn diagnostics(&self) -> Vec<PyDiagnostic> {
        self.warnings()
    }
    #[pyo3(signature=(cancel_token=None))]
    fn to_pdf<'py>(
        &self,
        py: Python<'py>,
        cancel_token: Option<PyRef<'_, PyCancelToken>>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let doc = self.inner.clone();
        let token = cancel_token.map(|t| t.inner.clone());
        let result = py.allow_threads(move || {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                doc.to_pdf_with_cancel(token.as_deref())
            }))
        });
        let bytes = result
            .map_err(|_| PyRuntimeError::new_err("PDF export failed internally"))?
            .map_err(pyerr)?;
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
    m.add_class::<PyRenderLimits>()?;
    m.add_class::<PyRenderStats>()?;
    m.add_class::<PyCancelToken>()?;
    m.add("RenderLimitError", m.py().get_type::<RenderLimitError>())?;
    m.add("MissingGlyphError", m.py().get_type::<MissingGlyphError>())?;
    m.add(
        "RenderCancelledError",
        m.py().get_type::<RenderCancelledError>(),
    )?;
    m.add_function(wrap_pyfunction!(capabilities, m)?)?;
    Ok(())
}
