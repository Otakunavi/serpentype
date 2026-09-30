//! Page-aware flow layout. The output display list is immutable input to PDF export.
use crate::css::{self, Color, MarginBox, PageStyle, Sheet, Style};
use crate::font::{FontData, FontRegistry};
use crate::html::{self, DocumentMetadata, Node};
use crate::{Diagnostic, Error, Result};
use base64::Engine as _;
use image::{GenericImageView, ImageDecoder};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;

const EPS: f32 = 0.02;
fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
    )
}
/// Hard bounds applied to one layout. The defaults target ordinary print documents.
#[derive(Clone, Debug)]
pub struct RenderLimits {
    pub max_pages: usize,
    pub max_input_bytes: usize,
    pub max_nodes: usize,
    pub max_css_rules: usize,
    pub max_image_pixels: u64,
    pub max_resource_bytes: usize,
    pub max_layout_iterations: usize,
    pub restrict_base_dir: bool,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_pages: 500,
            max_input_bytes: 20_000_000,
            max_nodes: 200_000,
            max_css_rules: 50_000,
            max_image_pixels: 50_000_000,
            max_resource_bytes: 100_000_000,
            max_layout_iterations: 1_000_000,
            restrict_base_dir: false,
        }
    }
}
fn cancelled(token: Option<&AtomicBool>) -> Result<()> {
    if token.is_some_and(|t| t.load(Ordering::Relaxed)) {
        Err(Error("render cancelled".into()))
    } else {
        Ok(())
    }
}

fn add_html_location(error: Error, html: &str) -> Error {
    let Some(hex) = error
        .0
        .strip_prefix("no registered font for U+")
        .and_then(|details| details.split_once(' ').map(|(hex, _)| hex))
    else {
        return error;
    };
    let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) else {
        return error;
    };
    let Some(offset) = html.find(ch) else {
        return error;
    };
    let before = &html[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before, |(_, current)| current)
        .chars()
        .count()
        + 1;
    Error(format!("{}; location <html> {line} {column}", error.0))
}
fn local_resource_path(base_dir: &Path, src: &str, restrict: bool) -> Result<PathBuf> {
    let candidate = base_dir.join(src);
    if !restrict {
        return Ok(candidate);
    }
    let allowed = base_dir
        .canonicalize()
        .map_err(|e| Error(format!("base_dir {}: {e}", base_dir.display())))?;
    let resolved = candidate
        .canonicalize()
        .map_err(|e| Error(format!("resource {}: {e}", candidate.display())))?;
    if !resolved.starts_with(&allowed) {
        return Err(Error(format!(
            "resource {} is outside base_dir {}",
            candidate.display(),
            base_dir.display()
        )));
    }
    Ok(resolved)
}
enum ImageFormat {
    Raster(image::ImageFormat),
    Svg,
}
fn data_image(src: &str, remaining: usize) -> Result<(Vec<u8>, ImageFormat)> {
    let (header, payload) = src
        .strip_prefix("data:")
        .and_then(|value| value.split_once(','))
        .ok_or_else(|| Error("invalid image data URI".into()))?;
    let mut parts = header.split(';');
    let format = match parts
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => ImageFormat::Raster(image::ImageFormat::Png),
        "image/jpeg" => ImageFormat::Raster(image::ImageFormat::Jpeg),
        "image/gif" => ImageFormat::Raster(image::ImageFormat::Gif),
        "image/webp" => ImageFormat::Raster(image::ImageFormat::WebP),
        "image/svg+xml" => ImageFormat::Svg,
        _ => return Err(Error("unsupported image data URI media type".into())),
    };
    if !parts.any(|part| part.eq_ignore_ascii_case("base64")) {
        return Err(Error("image data URI requires base64 encoding".into()));
    }
    if payload.len() > (remaining.saturating_mul(4) / 3).saturating_add(8) {
        return Err(Error("render limit exceeded: max_resource_bytes".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| Error(format!("invalid image data URI base64: {e}")))?;
    if bytes.len() > remaining {
        return Err(Error("render limit exceeded: max_resource_bytes".into()));
    }
    if matches!(format, ImageFormat::Raster(expected) if image::guess_format(&bytes).ok() != Some(expected))
    {
        return Err(Error(
            "image data URI media type does not match content".into(),
        ));
    }
    Ok((bytes, format))
}
/// A fixed RGB raster loaded at layout time.
#[derive(Debug)]
pub struct ImageData {
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
    pub jpeg: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub display_width: f32,
    pub display_height: f32,
    pub digest: [u8; 32],
}
fn render_svg(
    bytes: &[u8],
    digest: [u8; 32],
    dpi: f32,
    pixel_limit: u64,
    fonts: &FontRegistry,
) -> Result<ImageData> {
    use resvg::{tiny_skia, usvg};

    let xml = std::str::from_utf8(bytes).map_err(|e| Error(format!("SVG UTF-8: {e}")))?;
    let source =
        usvg::roxmltree::Document::parse(xml).map_err(|e| Error(format!("SVG XML parse: {e}")))?;
    for node in source.descendants().filter(|node| node.is_element()) {
        let tag = node.tag_name().name();
        if matches!(
            tag,
            "foreignObject"
                | "script"
                | "iframe"
                | "video"
                | "audio"
                | "canvas"
                | "animate"
                | "animateTransform"
                | "animateMotion"
                | "set"
        ) {
            return Err(Error(format!("SVG element is unsupported: {tag}")));
        }
        if tag == "use" {
            if let Some(href) = node
                .attributes()
                .find(|attribute| attribute.name() == "href")
                .map(|attribute| attribute.value())
            {
                if !href.starts_with('#') {
                    return Err(Error(format!(
                        "SVG external use reference is unsupported: {href}"
                    )));
                }
            }
        }
    }
    let has_text = source.descendants().any(|node| {
        node.tag_name().name() == "text"
            && node
                .descendants()
                .any(|child| child.is_text() && !child.text().unwrap_or("").trim().is_empty())
    });
    for node in source
        .descendants()
        .filter(|node| node.tag_name().name() == "image")
    {
        if let Some(href) = node
            .attributes()
            .find(|attribute| attribute.name() == "href")
            .map(|attribute| attribute.value())
        {
            if !href.starts_with("data:") {
                return Err(Error(format!(
                    "SVG external image resource is unsupported: {href}"
                )));
            }
            let (embedded, format) = data_image(href, bytes.len())?;
            if !matches!(format, ImageFormat::Raster(_)) {
                return Err(Error("nested SVG images are unsupported".into()));
            }
            let (width, height) = image::ImageReader::new(std::io::Cursor::new(&embedded))
                .with_guessed_format()
                .map_err(|e| Error(format!("SVG embedded image format: {e}")))?
                .into_dimensions()
                .map_err(|e| Error(format!("SVG embedded image dimensions: {e}")))?;
            if u64::from(width) * u64::from(height) > pixel_limit {
                return Err(Error("render limit exceeded: max_image_pixels".into()));
            }
        }
    }
    let mut options = usvg::Options {
        dpi: 96.0,
        font_family: "Noto Sans".into(),
        ..usvg::Options::default()
    };
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let font_sources = fonts.source_bytes()?;
    if has_text && font_sources.is_empty() {
        return Err(Error("SVG text requires a registered font".into()));
    }
    for text_node in source
        .descendants()
        .filter(|node| node.tag_name().name() == "text")
    {
        for value in text_node
            .descendants()
            .filter(|node| node.is_text())
            .filter_map(|node| node.text())
        {
            for ch in value.chars().filter(|ch| !ch.is_whitespace()) {
                fonts.resolve(&[], 400, "normal", ch).map_err(|_| {
                    Error(format!(
                        "SVG text has no registered glyph U+{:04X} ({ch})",
                        ch as u32
                    ))
                })?;
            }
        }
    }
    for font_source in font_sources {
        options.fontdb_mut().load_font_data(font_source);
    }
    let tree =
        usvg::Tree::from_data(bytes, &options).map_err(|e| Error(format!("SVG parse: {e}")))?;
    let intrinsic = tree.size();
    let scale = dpi / 96.0;
    let width = (intrinsic.width() * scale).ceil();
    let height = (intrinsic.height() * scale).ceil();
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height < 1.0
        || width > u32::MAX as f32
        || height > u32::MAX as f32
    {
        return Err(Error("SVG raster dimensions are invalid".into()));
    }
    let (width, height) = (width as u32, height as u32);
    if u64::from(width) * u64::from(height) > pixel_limit {
        return Err(Error("render limit exceeded: max_image_pixels".into()));
    }
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| Error("SVG raster allocation failed".into()))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    let mut alpha = Vec::with_capacity(width as usize * height as usize);
    for pixel in pixmap.data().as_chunks::<4>().0 {
        let a = pixel[3];
        for channel in &pixel[..3] {
            rgb.push(if a == 0 {
                0
            } else {
                (u16::from(*channel) * 255 / u16::from(a)).min(255) as u8
            });
        }
        alpha.push(a);
    }
    Ok(ImageData {
        rgb,
        alpha: alpha.iter().any(|a| *a != 255).then_some(alpha),
        jpeg: None,
        width,
        height,
        display_width: intrinsic.width() * 0.75,
        display_height: intrinsic.height() * 0.75,
        digest,
    })
}
impl ImageData {
    fn retained_bytes(&self) -> usize {
        self.rgb.len()
            + self.alpha.as_ref().map_or(0, Vec::len)
            + self.jpeg.as_ref().map_or(0, Vec::len)
    }
}
#[derive(Default)]
struct ImageCache {
    entries: HashMap<[u8; 32], Arc<ImageData>>,
    order: VecDeque<[u8; 32]>,
    bytes: usize,
    hits: usize,
    misses: usize,
}
/// Measurements for one layout and its most recent PDF export.
#[derive(Clone, Debug, Default)]
pub struct RenderStats {
    pub parse_ms: f64,
    pub css_cascade_ms: f64,
    pub shaping_ms: f64,
    pub layout_ms: f64,
    pub pdf_export_ms: f64,
    pub page_count: usize,
    pub dom_node_count: usize,
    pub glyph_count: usize,
    pub font_cache_hits: usize,
    pub font_cache_misses: usize,
    pub image_cache_hits: usize,
    pub image_cache_misses: usize,
    pub retained_bytes: usize,
    pub output_size: usize,
    pub diagnostic_count: usize,
}
const IMAGE_CACHE_LIMIT: usize = 32 * 1024 * 1024;
/// Positioned PDF primitives in top-left coordinates, measured in points.
#[derive(Clone, Debug)]
pub enum Item {
    Text {
        ch: char,
        glyph: u16,
        unicode: Option<Arc<str>>,
        natural_advance: f32,
        font: Arc<FontData>,
        x: f32,
        y: f32,
        size: f32,
        color: Color,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Option<Color>,
        stroke: Option<(f32, Color)>,
    },
    Image {
        data: Arc<ImageData>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    Link {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        target: String,
    },
    Destination {
        name: String,
        x: f32,
        y: f32,
    },
    BeginActualText(String),
    EndActualText,
}
fn shift_item(item: &mut Item, shift: f32) {
    translate_item(item, 0.0, shift);
}
fn translate_item(item: &mut Item, dx: f32, dy: f32) {
    match item {
        Item::Text { x, y, .. }
        | Item::Rect { x, y, .. }
        | Item::Image { x, y, .. }
        | Item::Link { x, y, .. }
        | Item::Destination { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Item::BeginActualText(_) | Item::EndActualText => {}
    }
}
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub items: Vec<Item>,
    pub style: PageStyle,
    pub name: Option<String>,
}
/// Fully paginated document. Cloning shares immutable fonts and images.
#[derive(Clone, Debug)]
pub struct PreparedDocument {
    pub pages: Vec<Page>,
    pub page_style: PageStyle,
    pub warnings: Vec<Diagnostic>,
    pub stats: Arc<Mutex<RenderStats>>,
    pub metadata: DocumentMetadata,
}
impl PreparedDocument {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    pub fn to_pdf(&self) -> Result<Vec<u8>> {
        self.to_pdf_with_cancel(None)
    }
    pub fn to_pdf_with_cancel(&self, token: Option<&AtomicBool>) -> Result<Vec<u8>> {
        let start = Instant::now();
        let bytes = crate::pdf::export_with_cancel(self, token)?;
        if let Ok(mut stats) = self.stats.lock() {
            stats.pdf_export_ms = start.elapsed().as_secs_f64() * 1000.0;
            stats.output_size = bytes.len();
        }
        Ok(bytes)
    }
}
/// Reusable renderer. Each call uses an isolated registration snapshot.
#[derive(Clone)]
pub struct Renderer {
    pub fonts: FontRegistry,
    pub base_dir: PathBuf,
    pub strict: bool,
    pub limits: RenderLimits,
    pub experimental_shaping: bool,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub svg_dpi: f32,
    pub presentational_hints: bool,
    images: Arc<Mutex<ImageCache>>,
}
impl Renderer {
    /// Create a renderer with an explicit font registry and local-resource base directory.
    pub fn new(fonts: FontRegistry, base_dir: PathBuf, strict: bool) -> Self {
        Self {
            fonts,
            base_dir,
            strict,
            limits: RenderLimits::default(),
            experimental_shaping: false,
            synthetic_bold: false,
            synthetic_italic: false,
            svg_dpi: 144.0,
            presentational_hints: false,
            images: Arc::new(Mutex::new(ImageCache::default())),
        }
    }
    /// Parse, measure and paginate a document without creating PDF bytes.
    pub fn layout(&self, html: &str, css_text: &str) -> Result<PreparedDocument> {
        self.layout_with_cancel(html, css_text, None)
    }
    pub fn layout_with_cancel(
        &self,
        html: &str,
        css_text: &str,
        token: Option<Arc<AtomicBool>>,
    ) -> Result<PreparedDocument> {
        cancelled(token.as_deref())?;
        if html.len().saturating_add(css_text.len()) > self.limits.max_input_bytes {
            return Err(Error("render limit exceeded: max_input_bytes".into()));
        }
        if self.limits.max_pages == 0 {
            return Err(Error("render limit exceeded: max_pages".into()));
        }
        let initial_font_cache = self.fonts.stats();
        let (initial_image_hits, initial_image_misses) = {
            let cache = self.images.lock().map_err(|e| Error(e.to_string()))?;
            (cache.hits, cache.misses)
        };
        let css_start = Instant::now();
        let all_css = format!("{}\n{}", html::embedded_css(html), css_text);
        let sheet = css::parse(&all_css, self.strict)?;
        let css_cascade_ms = css_start.elapsed().as_secs_f64() * 1000.0;
        if sheet.rules.len() > self.limits.max_css_rules {
            return Err(Error("render limit exceeded: max_css_rules".into()));
        }
        if sheet.page.width <= sheet.page.margin[1] + sheet.page.margin[3]
            || sheet.page.height <= sheet.page.margin[0] + sheet.page.margin[2]
            || sheet.page.margin.iter().any(|v| *v < 0.0)
        {
            return Err(Error(
                "@page margins leave no positive content rectangle".into(),
            ));
        }
        let fonts = self.fonts.snapshot();
        fonts.set_synthetic(self.synthetic_bold, self.synthetic_italic);
        let mut font_resource_bytes = 0usize;
        // Local @font-face files are snapshotted before layout, never at PDF export.
        for face in &sheet.faces {
            let src = face.src.trim();
            let path = src
                .strip_prefix("url(")
                .and_then(|s| s.strip_suffix(')'))
                .ok_or_else(|| Error(format!("unsupported @font-face src: {src}")))?
                .trim()
                .trim_matches(['\'', '"']);
            if path.contains("://") || path.starts_with("data:") {
                return Err(Error("@font-face supports local file URLs only".into()));
            }
            let resolved =
                local_resource_path(&self.base_dir, path, self.limits.restrict_base_dir)?;
            let size = std::fs::metadata(&resolved)
                .map_err(|e| Error(format!("font {}: {e}", resolved.display())))?
                .len() as usize;
            font_resource_bytes = font_resource_bytes.saturating_add(size);
            if font_resource_bytes > self.limits.max_resource_bytes {
                return Err(Error("render limit exceeded: max_resource_bytes".into()));
            }
            cancelled(token.as_deref())?;
            fonts.register_file(
                &resolved.to_string_lossy(),
                &face.family,
                face.weight,
                &face.style,
            )?;
        }
        let parse_start = Instant::now();
        let (root, warnings, metadata) = html::parse_with_metadata_and_hints(
            html,
            &sheet,
            self.strict,
            self.presentational_hints,
        )?;
        let parse_ms = parse_start.elapsed().as_secs_f64() * 1000.0;
        fn count_tree(node: &Node) -> (usize, usize) {
            node.children.iter().fold(
                (1usize, node.text.chars().count().saturating_add(1)),
                |(nodes, work), child| {
                    let (cn, cw) = count_tree(child);
                    (nodes.saturating_add(cn), work.saturating_add(cw))
                },
            )
        }
        let (node_count, estimated_work) = count_tree(&root);
        if node_count > self.limits.max_nodes {
            return Err(Error("render limit exceeded: max_nodes".into()));
        }
        if estimated_work > self.limits.max_layout_iterations {
            return Err(Error("render limit exceeded: max_layout_iterations".into()));
        }
        cancelled(token.as_deref())?;
        for page in sheet.named_pages.values() {
            if page.width <= page.margin[1] + page.margin[3]
                || page.height <= page.margin[0] + page.margin[2]
                || page.margin.iter().any(|v| *v < 0.0)
            {
                return Err(Error(
                    "named @page margins leave no positive content rectangle".into(),
                ));
            }
        }
        let layout_start = Instant::now();
        let mut flow = Flow::new(
            sheet.clone(),
            fonts,
            self.base_dir.clone(),
            self.images.clone(),
            warnings,
            self.limits.clone(),
            token,
        );
        flow.experimental_shaping = self.experimental_shaping;
        flow.svg_dpi = self.svg_dpi;
        flow.resource_bytes = font_resource_bytes;
        for child in &root.children {
            flow.node(child)
                .map_err(|error| add_html_location(error, html))?;
        }
        flow.render_margin_boxes()?;
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let mut seen: HashMap<(&'static str, String), usize> = HashMap::new();
        for warning in flow.warnings {
            let key = (warning.code, warning.message.clone());
            if let Some(&index) = seen.get(&key) {
                diagnostics[index].occurrences += warning.occurrences;
            } else {
                seen.insert(key, diagnostics.len());
                diagnostics.push(warning);
            }
        }
        let font_cache = self.fonts.stats();
        let image_cache = self.images.lock().map_err(|e| Error(e.to_string()))?;
        let stats = RenderStats {
            parse_ms,
            css_cascade_ms,
            shaping_ms: flow.shaping_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            layout_ms: layout_start.elapsed().as_secs_f64() * 1000.0,
            page_count: flow.pages.len(),
            dom_node_count: node_count,
            glyph_count: flow
                .pages
                .iter()
                .flat_map(|p| &p.items)
                .filter(|item| matches!(item, Item::Text { .. }))
                .count(),
            font_cache_hits: font_cache.hits.saturating_sub(initial_font_cache.hits),
            font_cache_misses: font_cache.misses.saturating_sub(initial_font_cache.misses),
            image_cache_hits: image_cache.hits.saturating_sub(initial_image_hits),
            image_cache_misses: image_cache.misses.saturating_sub(initial_image_misses),
            retained_bytes: font_cache.bytes.saturating_add(image_cache.bytes),
            diagnostic_count: diagnostics.len(),
            ..RenderStats::default()
        };
        Ok(PreparedDocument {
            pages: flow.pages,
            page_style: sheet.page,
            warnings: diagnostics,
            stats: Arc::new(Mutex::new(stats)),
            metadata,
        })
    }
}

#[derive(Clone)]
struct Glyph {
    ch: char,
    id: u16,
    font: Arc<FontData>,
    advance: f32,
    natural_advance: f32,
    x_offset: f32,
    y_offset: f32,
    unicode: Option<Arc<str>>,
    size: f32,
    color: Color,
    shift: f32,
    decoration: TextDecoration,
    href: Option<Arc<str>>,
    preserve_space: bool,
    visible: bool,
    soft_hyphen: bool,
    soft_hyphen_advance: f32,
    inline_box: Option<Arc<InlineBox>>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextDecoration {
    None,
    Underline,
    LineThrough,
}
struct InlineRun {
    text: String,
    style: Style,
    href: Option<Arc<str>>,
    inline_box: Option<Node>,
}
#[derive(Clone)]
struct InlineBox {
    lines: Vec<Line>,
    style: Style,
    width: f32,
    height: f32,
    inner_width: f32,
    href: Option<Arc<str>>,
    destination: Option<String>,
}
#[derive(Clone, Default)]
struct Line {
    glyphs: Vec<Glyph>,
    width: f32,
    height: f32,
    baseline: f32,
    actual_text: Option<String>,
}
fn push_line(lines: &mut Vec<Line>, mut glyphs: Vec<Glyph>, default_height: f32) {
    while glyphs
        .last()
        .is_some_and(|g| g.ch == ' ' && !g.preserve_space)
    {
        glyphs.pop();
    }
    let width = glyphs.iter().map(|g| g.advance).sum();
    let baseline = glyphs
        .iter()
        .map(|g| {
            g.inline_box
                .as_ref()
                .map_or(g.size * 0.9 + g.shift, |inline| {
                    inline.style.margin[0] + inline.height + g.shift
                })
        })
        .fold(default_height * 0.75, f32::max);
    let descent = glyphs
        .iter()
        .map(|g| {
            g.inline_box
                .as_ref()
                .map_or(g.size * 0.3 - g.shift, |inline| {
                    inline.style.margin[2] - g.shift
                })
        })
        .fold(default_height * 0.25, f32::max);
    let height = (baseline + descent).max(default_height);
    lines.push(Line {
        glyphs,
        width,
        height,
        baseline,
        actual_text: None,
    });
}

fn break_at_soft_hyphen(
    current: &mut Vec<Glyph>,
    lines: &mut Vec<Line>,
    default_height: f32,
) -> Option<f32> {
    let index = current.iter().rposition(|glyph| glyph.soft_hyphen)?;
    let trailing = current.split_off(index + 1);
    let hyphen = current.last_mut()?;
    hyphen.advance = hyphen.soft_hyphen_advance;
    hyphen.unicode = None;
    push_line(lines, std::mem::take(current), default_height);
    *current = trailing;
    Some(current.iter().map(|glyph| glyph.advance).sum())
}

fn last_break_opportunity(glyphs: &[Glyph]) -> Option<(usize, bool)> {
    glyphs.iter().enumerate().rev().find_map(|(index, glyph)| {
        (glyph.ch == ' ' || glyph.soft_hyphen).then_some((index, glyph.soft_hyphen))
    })
}

fn soft_hyphen_glyph(
    style: &Style,
    fonts: &FontRegistry,
    href: Option<Arc<str>>,
    preserve_space: bool,
) -> Result<Glyph> {
    let font = fonts.resolve_with_stretch(
        &style.family,
        style.weight,
        &style.font_style,
        style.font_stretch,
        '-',
    )?;
    let (id, units) = font.glyph('-')?;
    let natural_advance = units as f32 * style.font_size / font.units_per_em as f32;
    Ok(Glyph {
        ch: '-',
        id,
        font,
        advance: 0.0,
        natural_advance,
        x_offset: 0.0,
        y_offset: 0.0,
        unicode: Some(Arc::<str>::from("")),
        size: style.font_size,
        color: style.color,
        shift: 0.0,
        decoration: TextDecoration::None,
        href,
        preserve_space,
        visible: style.visibility == "visible",
        soft_hyphen: true,
        soft_hyphen_advance: natural_advance + style.letter_spacing,
        inline_box: None,
    })
}
fn inline_runs(
    node: &Node,
    inherited_href: Option<Arc<str>>,
    out: &mut Vec<InlineRun>,
    root: bool,
) {
    let href = node
        .attr("href")
        .filter(|target| {
            matches!(
                target
                    .split_once(':')
                    .map(|(scheme, _)| scheme.to_ascii_lowercase())
                    .as_deref(),
                Some("http" | "https" | "mailto")
            ) || target.starts_with('#')
        })
        .map(Arc::<str>::from)
        .or(inherited_href);
    if !root && node.tag != "#text" && node.style.display == "inline-block" {
        out.push(InlineRun {
            text: String::new(),
            style: node.style.clone(),
            href,
            inline_box: Some(node.clone()),
        });
        return;
    }
    if node.tag == "br" {
        out.push(InlineRun {
            text: "\u{000b}".into(),
            style: node.style.clone(),
            href,
            inline_box: None,
        });
        return;
    }
    if !node.text.is_empty() {
        out.push(InlineRun {
            text: node.text.clone(),
            style: node.style.clone(),
            href: href.clone(),
            inline_box: None,
        });
    }
    for child in &node.children {
        inline_runs(child, href.clone(), out, false);
    }
}

fn inline_box_glyph(
    node: &Node,
    href: Option<Arc<str>>,
    containing_width: f32,
    fonts: &FontRegistry,
    token: Option<&AtomicBool>,
    experimental_shaping: bool,
    shaping_ns: &AtomicU64,
) -> Result<Glyph> {
    let style = &node.style;
    let mut content_node = node.clone();
    if href.is_some() {
        content_node.attrs.remove("href");
    }
    let horizontal_edges = style.padding[1] + style.padding[3] + 2.0 * style.border_width;
    let horizontal_margins = style.margin[1] + style.margin[3];
    let available_outer = containing_width - horizontal_margins;
    if available_outer <= horizontal_edges + EPS {
        return Err(Error("inline-block has no usable width".into()));
    }
    let declared = resolved_dimension(style.width, style.width_percent, containing_width);
    let minimum = resolved_dimension(style.min_width, style.min_width_percent, containing_width);
    let maximum = resolved_dimension(style.max_width, style.max_width_percent, containing_width);
    let sizing_width = if let Some(declared) = declared {
        declared
    } else {
        let intrinsic = lines_for(
            &content_node,
            1_000_000.0,
            0.0,
            fonts,
            token,
            experimental_shaping,
            shaping_ns,
        )?
        .iter()
        .map(|line| line.width)
        .fold(0.0, f32::max);
        if style.box_sizing == "border-box" {
            (intrinsic + horizontal_edges).min(available_outer)
        } else {
            intrinsic.min(available_outer - horizontal_edges)
        }
    };
    let sizing_width = constrained_dimension(sizing_width, minimum, maximum);
    let width = if style.box_sizing == "border-box" {
        sizing_width
    } else {
        sizing_width + horizontal_edges
    };
    let inner_width = width - horizontal_edges;
    if inner_width <= 0.0 {
        return Err(Error("inline-block has no usable content width".into()));
    }
    let lines = lines_for(
        &content_node,
        inner_width,
        0.0,
        fonts,
        token,
        experimental_shaping,
        shaping_ns,
    )?;
    let body_height: f32 = lines.iter().map(|line| line.height).sum();
    let vertical_edges = style.padding[0] + style.padding[2] + 2.0 * style.border_width;
    let natural_height = body_height + vertical_edges;
    let declared_height = resolved_dimension(style.height, style.height_percent, natural_height);
    let minimum_height =
        resolved_dimension(style.min_height, style.min_height_percent, natural_height);
    let maximum_height =
        resolved_dimension(style.max_height, style.max_height_percent, natural_height);
    let sizing_height = declared_height.unwrap_or_else(|| {
        if style.box_sizing == "border-box" {
            natural_height
        } else {
            body_height
        }
    });
    let sizing_height = constrained_dimension(sizing_height, minimum_height, maximum_height);
    let requested_height = if style.box_sizing == "border-box" {
        sizing_height
    } else {
        sizing_height + vertical_edges
    };
    let height = natural_height.max(requested_height);
    let font = lines
        .iter()
        .flat_map(|line| &line.glyphs)
        .find(|glyph| glyph.inline_box.is_none())
        .map(|glyph| glyph.font.clone())
        .map(Ok)
        .unwrap_or_else(|| {
            fonts.resolve_with_stretch(
                &style.family,
                style.weight,
                &style.font_style,
                style.font_stretch,
                ' ',
            )
        })?;
    let (id, _) = font.glyph(' ')?;
    let shift = match style.vertical_align.as_str() {
        "super" | "top" => style.font_size * 0.35,
        "sub" | "bottom" => -style.font_size * 0.2,
        "middle" => style.font_size * 0.1,
        _ => 0.0,
    };
    let advance = style.margin[3] + width + style.margin[1];
    Ok(Glyph {
        ch: '\u{fffc}',
        id,
        font,
        advance,
        natural_advance: advance,
        x_offset: 0.0,
        y_offset: 0.0,
        unicode: Some(Arc::<str>::from(node.plain_text())),
        size: style.font_size,
        color: style.color,
        shift,
        decoration: TextDecoration::None,
        href: None,
        preserve_space: false,
        visible: style.visibility == "visible",
        soft_hyphen: false,
        soft_hyphen_advance: 0.0,
        inline_box: Some(Arc::new(InlineBox {
            lines,
            style: style.clone(),
            width,
            height,
            inner_width,
            href,
            destination: node.attr("id").map(str::to_owned),
        })),
    })
}

fn place_atomic_glyph(
    glyph: Glyph,
    lines: &mut Vec<Line>,
    current: &mut Vec<Glyph>,
    current_width: &mut f32,
    width: f32,
    first_indent: f32,
    default_height: f32,
) -> Result<()> {
    let available = if lines.is_empty() {
        width - first_indent
    } else {
        width
    };
    if *current_width + glyph.advance > available + EPS && !current.is_empty() {
        push_line(lines, std::mem::take(current), default_height);
        *current_width = 0.0;
    }
    let available = if lines.is_empty() {
        width - first_indent
    } else {
        width
    };
    if glyph.advance > available + EPS {
        return Err(Error(format!(
            "inline-block width {:.3} exceeds available line width {:.3}",
            glyph.advance, available
        )));
    }
    *current_width += glyph.advance;
    current.push(glyph);
    Ok(())
}
fn lines_for(
    node: &Node,
    width: f32,
    first_indent: f32,
    fonts: &FontRegistry,
    token: Option<&AtomicBool>,
    experimental_shaping: bool,
    shaping_ns: &AtomicU64,
) -> Result<Vec<Line>> {
    if experimental_shaping {
        let start = Instant::now();
        let result = lines_for_shaped(node, width, first_indent, fonts, token);
        shaping_ns.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        return result;
    }
    if width - first_indent <= 0.0 {
        return Err(Error("text-indent leaves no usable first line".into()));
    }
    let mut runs = Vec::new();
    inline_runs(node, None, &mut runs, true);
    let mut lines = Vec::new();
    let mut current = Vec::<Glyph>::new();
    let mut current_width = 0.0;
    let default_height = node.style.line_height.max(node.style.font_size);
    let mut previous_space = false;
    let mut character_index = 0usize;
    for run in runs {
        let style = &run.style;
        if let Some(inline) = &run.inline_box {
            let glyph = inline_box_glyph(
                inline,
                run.href.clone(),
                width,
                fonts,
                token,
                experimental_shaping,
                shaping_ns,
            )?;
            place_atomic_glyph(
                glyph,
                &mut lines,
                &mut current,
                &mut current_width,
                width,
                first_indent,
                default_height,
            )?;
            previous_space = false;
            continue;
        }
        for raw in run.text.chars() {
            if character_index.is_multiple_of(1024) {
                cancelled(token)?;
            }
            character_index += 1;
            let preserve = matches!(style.white_space.as_str(), "pre" | "pre-wrap");
            if raw == '\u{00ad}' {
                previous_space = false;
                if style.hyphens == "manual" {
                    current.push(soft_hyphen_glyph(style, fonts, run.href.clone(), preserve)?);
                }
                continue;
            }
            if raw == '\u{000b}' || (raw == '\n' && preserve) {
                push_line(&mut lines, std::mem::take(&mut current), default_height);
                current_width = 0.0;
                previous_space = false;
                continue;
            }
            let ch = if raw.is_whitespace() && raw != '\u{00a0}' {
                ' '
            } else {
                raw
            };
            if ch == ' ' && !preserve && (previous_space || current.is_empty()) {
                continue;
            }
            previous_space = ch == ' ' && !preserve;
            let font = fonts.resolve_with_stretch(
                &style.family,
                style.weight,
                &style.font_style,
                style.font_stretch,
                ch,
            )?;
            let (id, units) = font.glyph(ch)?;
            let natural_advance = units as f32 * style.font_size / font.units_per_em as f32;
            let advance = natural_advance
                + style.letter_spacing
                + if ch == ' ' { style.word_spacing } else { 0.0 };
            let shift = match style.vertical_align.as_str() {
                "super" | "top" => style.font_size * 0.35,
                "sub" | "bottom" => -style.font_size * 0.2,
                "middle" => style.font_size * 0.1,
                _ => 0.0,
            };
            let glyph = Glyph {
                ch,
                id,
                font,
                advance,
                natural_advance,
                x_offset: 0.0,
                y_offset: 0.0,
                unicode: None,
                size: style.font_size,
                color: style.color,
                shift,
                decoration: match style.text_decoration.as_str() {
                    "underline" => TextDecoration::Underline,
                    "line-through" => TextDecoration::LineThrough,
                    _ => TextDecoration::None,
                },
                href: run.href.clone(),
                preserve_space: preserve,
                visible: style.visibility == "visible",
                soft_hyphen: false,
                soft_hyphen_advance: 0.0,
                inline_box: None,
            };
            let available = if lines.is_empty() {
                width - first_indent
            } else {
                width
            };
            if current_width + advance > available + EPS
                && !current.is_empty()
                && !matches!(style.white_space.as_str(), "nowrap" | "pre")
            {
                // Prefer the last word boundary; an overwide word falls back to glyph breaks.
                match last_break_opportunity(&current) {
                    Some((_, true)) => {
                        current_width =
                            break_at_soft_hyphen(&mut current, &mut lines, default_height)
                                .unwrap_or(0.0);
                    }
                    Some((last_space, false)) => {
                        let trailing = current.split_off(last_space + 1);
                        current.pop();
                        push_line(&mut lines, std::mem::take(&mut current), default_height);
                        current = trailing;
                        current_width = current.iter().map(|g| g.advance).sum();
                    }
                    None => {
                        if style.overflow_wrap != "normal" || style.word_break == "break-all" {
                            push_line(&mut lines, std::mem::take(&mut current), default_height);
                            current_width = 0.0;
                        }
                    }
                }
            }
            if ch != ' ' || !current.is_empty() || preserve {
                current_width += advance;
                current.push(glyph);
            }
        }
    }
    if !current.is_empty() || lines.is_empty() {
        push_line(&mut lines, current, default_height);
    }
    if lines
        .iter()
        .enumerate()
        .any(|(index, line)| line.width > width - if index == 0 { first_indent } else { 0.0 } + EPS)
    {
        return Err(Error("text line exceeds available width".into()));
    }
    Ok(lines)
}

enum ShapedUnit {
    Break,
    Glyph(Glyph, bool, bool),
}

fn shape_segment(
    segment: &str,
    font: &Arc<FontData>,
    style: &Style,
    href: &Option<Arc<str>>,
    units: &mut Vec<ShapedUnit>,
) -> Result<()> {
    if segment.is_empty() {
        return Ok(());
    }
    let scale = style.font_size / font.units_per_em as f32;
    let shift = match style.vertical_align.as_str() {
        "super" | "top" => style.font_size * 0.35,
        "sub" | "bottom" => -style.font_size * 0.2,
        "middle" => style.font_size * 0.1,
        _ => 0.0,
    };
    let decoration = match style.text_decoration.as_str() {
        "underline" => TextDecoration::Underline,
        "line-through" => TextDecoration::LineThrough,
        _ => TextDecoration::None,
    };
    let can_wrap = !matches!(style.white_space.as_str(), "nowrap" | "pre");
    let break_word = style.overflow_wrap != "normal" || style.word_break == "break-all";
    let mut shaped_glyphs = font.shape(segment)?;
    shaped_glyphs.sort_by_key(|glyph| glyph.cluster);
    for shaped in shaped_glyphs {
        if shaped.id == 0 {
            return Err(Error(format!(
                "shaping produced a missing glyph for {}",
                shaped.unicode
            )));
        }
        let ch = shaped.unicode.chars().next().unwrap_or('\0');
        let natural_advance = shaped.advance as f32 * scale;
        let unicode = if shaped.unicode.chars().count() == 1 && shaped.unicode.starts_with(ch) {
            None
        } else {
            Some(Arc::<str>::from(shaped.unicode))
        };
        units.push(ShapedUnit::Glyph(
            Glyph {
                ch,
                id: shaped.id,
                font: font.clone(),
                advance: natural_advance + style.letter_spacing,
                natural_advance,
                x_offset: shaped.x_offset as f32 * scale,
                y_offset: shaped.y_offset as f32 * scale,
                unicode,
                size: style.font_size,
                color: style.color,
                shift,
                decoration,
                href: href.clone(),
                preserve_space: false,
                visible: style.visibility == "visible",
                soft_hyphen: false,
                soft_hyphen_advance: 0.0,
                inline_box: None,
            },
            can_wrap,
            break_word,
        ));
    }
    Ok(())
}

fn reorder_bidi_line(line: &mut Line) {
    let mut logical = String::new();
    let mut visible_logical = String::new();
    let mut glyph_offsets = Vec::with_capacity(line.glyphs.len());
    let mut last_offset = 0usize;
    for glyph in &line.glyphs {
        let value = glyph
            .unicode
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| glyph.ch.to_string());
        if value.is_empty() {
            glyph_offsets.push(last_offset);
        } else {
            last_offset = logical.len();
            glyph_offsets.push(last_offset);
            logical.push_str(&value);
            if glyph.visible {
                visible_logical.extend(value.chars().filter(|ch| !is_bidi_control(*ch)));
            }
        }
    }
    if logical.is_empty() {
        return;
    }
    let bidi = unicode_bidi::BidiInfo::new(&logical, None);
    if !bidi.has_rtl() {
        line.glyphs.retain(|glyph| glyph.id != 0);
        return;
    }
    if !visible_logical.is_empty() {
        line.actual_text = Some(visible_logical);
    }
    let Some(paragraph) = bidi.paragraphs.first() else {
        return;
    };
    let levels = bidi.reordered_levels(paragraph, 0..logical.len());
    let glyph_levels = glyph_offsets
        .iter()
        .map(|offset| levels.get(*offset).copied().unwrap_or(paragraph.level))
        .collect::<Vec<_>>();
    let visual_order = unicode_bidi::BidiInfo::reorder_visual(&glyph_levels);
    let logical_glyphs = std::mem::take(&mut line.glyphs);
    line.glyphs = visual_order
        .into_iter()
        .map(|index| logical_glyphs[index].clone())
        .filter(|glyph| glyph.id != 0)
        .collect();
}

fn lines_for_shaped(
    node: &Node,
    width: f32,
    first_indent: f32,
    fonts: &FontRegistry,
    token: Option<&AtomicBool>,
) -> Result<Vec<Line>> {
    if width - first_indent <= 0.0 {
        return Err(Error("text-indent leaves no usable first line".into()));
    }
    let mut runs = Vec::new();
    inline_runs(node, None, &mut runs, true);
    let mut units = Vec::new();
    let mut previous_space = false;
    let mut character_index = 0usize;
    let mut preceding_rtl = false;
    for run in runs {
        let style = &run.style;
        let preserve = matches!(style.white_space.as_str(), "pre" | "pre-wrap");
        if let Some(inline) = &run.inline_box {
            units.push(ShapedUnit::Glyph(
                inline_box_glyph(
                    inline,
                    run.href.clone(),
                    width,
                    fonts,
                    token,
                    true,
                    &AtomicU64::new(0),
                )?,
                true,
                false,
            ));
            previous_space = false;
            preceding_rtl = false;
            continue;
        }
        let mut segment = String::new();
        let mut segment_font: Option<Arc<FontData>> = None;
        let mut segment_rtl: Option<bool> = None;
        let clusters = if run.text.is_ascii() {
            (0..run.text.len())
                .map(|index| &run.text[index..index + 1])
                .collect::<Vec<_>>()
        } else {
            UnicodeSegmentation::graphemes(run.text.as_str(), true).collect::<Vec<_>>()
        };
        for (cluster_index, raw) in clusters.iter().copied().enumerate() {
            if character_index.is_multiple_of(1024) {
                cancelled(token)?;
            }
            character_index += raw.chars().count();
            if raw == "\u{00ad}" {
                if let Some(font) = segment_font.take() {
                    shape_segment(&segment, &font, style, &run.href, &mut units)?;
                    segment.clear();
                }
                segment_rtl = None;
                previous_space = false;
                if style.hyphens == "manual" {
                    units.push(ShapedUnit::Glyph(
                        soft_hyphen_glyph(style, fonts, run.href.clone(), preserve)?,
                        true,
                        false,
                    ));
                }
                continue;
            }
            if raw == "\u{000b}" || (raw == "\n" && preserve) {
                if let Some(font) = segment_font.take() {
                    shape_segment(&segment, &font, style, &run.href, &mut units)?;
                    segment.clear();
                }
                segment_rtl = None;
                units.push(ShapedUnit::Break);
                previous_space = false;
                preceding_rtl = false;
                continue;
            }
            let cluster = if raw.chars().all(char::is_whitespace) && !raw.contains('\u{00a0}') {
                " "
            } else {
                raw
            };
            let ch = cluster.chars().next().unwrap_or(' ');
            if cluster.chars().all(is_bidi_control) {
                let preceding_font = segment_font.take();
                if let Some(font) = &preceding_font {
                    shape_segment(&segment, font, style, &run.href, &mut units)?;
                    segment.clear();
                }
                segment_rtl = None;
                let font = if let Some(font) = preceding_font {
                    font
                } else {
                    fonts.resolve_with_stretch(
                        &style.family,
                        style.weight,
                        &style.font_style,
                        style.font_stretch,
                        ' ',
                    )?
                };
                units.push(ShapedUnit::Glyph(
                    Glyph {
                        ch,
                        id: 0,
                        font,
                        advance: 0.0,
                        natural_advance: 0.0,
                        x_offset: 0.0,
                        y_offset: 0.0,
                        unicode: None,
                        size: style.font_size,
                        color: style.color,
                        shift: 0.0,
                        decoration: TextDecoration::None,
                        href: None,
                        preserve_space: false,
                        visible: false,
                        soft_hyphen: false,
                        soft_hyphen_advance: 0.0,
                        inline_box: None,
                    },
                    false,
                    false,
                ));
                continue;
            }
            if ch == ' ' {
                let preceding_font = segment_font.take();
                if let Some(font) = &preceding_font {
                    shape_segment(&segment, font, style, &run.href, &mut units)?;
                    segment.clear();
                }
                segment_rtl = None;
                if !preserve && previous_space {
                    continue;
                }
                previous_space = !preserve;
                let following_ltr = clusters[cluster_index + 1..]
                    .iter()
                    .flat_map(|cluster| cluster.chars())
                    .find(|next| {
                        matches!(
                            unicode_bidi::bidi_class(*next),
                            unicode_bidi::BidiClass::L
                                | unicode_bidi::BidiClass::R
                                | unicode_bidi::BidiClass::AL
                        )
                    })
                    .filter(|next| unicode_bidi::bidi_class(*next) == unicode_bidi::BidiClass::L);
                let font = if let (true, Some(following)) = (preceding_rtl, following_ltr) {
                    let next_font = fonts.resolve_with_stretch(
                        &style.family,
                        style.weight,
                        &style.font_style,
                        style.font_stretch,
                        following,
                    )?;
                    if next_font.has_glyph(ch) {
                        next_font
                    } else {
                        fonts.resolve_with_stretch(
                            &style.family,
                            style.weight,
                            &style.font_style,
                            style.font_stretch,
                            ch,
                        )?
                    }
                } else if let Some(font) = preceding_font.filter(|font| font.has_glyph(ch)) {
                    font
                } else {
                    fonts.resolve_with_stretch(
                        &style.family,
                        style.weight,
                        &style.font_style,
                        style.font_stretch,
                        ch,
                    )?
                };
                let (id, advance_units) = font.glyph(ch)?;
                let natural_advance =
                    advance_units as f32 * style.font_size / font.units_per_em as f32;
                units.push(ShapedUnit::Glyph(
                    Glyph {
                        ch,
                        id,
                        font,
                        advance: natural_advance + style.letter_spacing + style.word_spacing,
                        natural_advance,
                        x_offset: 0.0,
                        y_offset: 0.0,
                        unicode: None,
                        size: style.font_size,
                        color: style.color,
                        shift: 0.0,
                        decoration: TextDecoration::None,
                        href: run.href.clone(),
                        preserve_space: preserve,
                        visible: style.visibility == "visible",
                        soft_hyphen: false,
                        soft_hyphen_advance: 0.0,
                        inline_box: None,
                    },
                    !matches!(style.white_space.as_str(), "nowrap" | "pre"),
                    style.overflow_wrap != "normal" || style.word_break == "break-all",
                ));
                continue;
            }
            previous_space = false;
            let direction =
                cluster
                    .chars()
                    .find_map(|value| match unicode_bidi::bidi_class(value) {
                        unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL => Some(true),
                        unicode_bidi::BidiClass::L => Some(false),
                        _ => None,
                    });
            if let Some(rtl) = direction {
                preceding_rtl = rtl;
                if segment_rtl.is_some_and(|existing| existing != rtl) {
                    if let Some(font) = segment_font.take() {
                        shape_segment(&segment, &font, style, &run.href, &mut units)?;
                        segment.clear();
                    }
                }
                segment_rtl = Some(rtl);
            }
            let font = if cluster.len() == ch.len_utf8() {
                fonts.resolve_with_stretch(
                    &style.family,
                    style.weight,
                    &style.font_style,
                    style.font_stretch,
                    ch,
                )?
            } else {
                fonts.resolve_cluster_with_stretch(
                    &style.family,
                    style.weight,
                    &style.font_style,
                    style.font_stretch,
                    cluster,
                )?
            };
            if segment_font
                .as_ref()
                .is_some_and(|selected| selected.digest != font.digest)
            {
                if let Some(selected) = segment_font.take() {
                    shape_segment(&segment, &selected, style, &run.href, &mut units)?;
                    segment.clear();
                }
            }
            segment_font = Some(font);
            segment.push_str(cluster);
        }
        if let Some(font) = segment_font {
            shape_segment(&segment, &font, style, &run.href, &mut units)?;
        }
    }
    let default_height = node.style.line_height.max(node.style.font_size);
    let mut lines = Vec::new();
    let mut current = Vec::<Glyph>::new();
    let mut current_width = 0.0;
    for unit in units {
        let ShapedUnit::Glyph(glyph, can_wrap, break_word) = unit else {
            push_line(&mut lines, std::mem::take(&mut current), default_height);
            current_width = 0.0;
            continue;
        };
        if glyph.ch == ' ' && current.is_empty() && !glyph.preserve_space {
            continue;
        }
        let available = if lines.is_empty() {
            width - first_indent
        } else {
            width
        };
        if current_width + glyph.advance > available + EPS && !current.is_empty() && can_wrap {
            match last_break_opportunity(&current) {
                Some((_, true)) => {
                    current_width = break_at_soft_hyphen(&mut current, &mut lines, default_height)
                        .unwrap_or(0.0);
                }
                Some((last_space, false)) => {
                    let trailing = current.split_off(last_space + 1);
                    current.pop();
                    push_line(&mut lines, std::mem::take(&mut current), default_height);
                    current = trailing;
                    current_width = current.iter().map(|item| item.advance).sum();
                }
                None if break_word => {
                    push_line(&mut lines, std::mem::take(&mut current), default_height);
                    current_width = 0.0;
                }
                None => {}
            }
        }
        current_width += glyph.advance;
        current.push(glyph);
    }
    if !current.is_empty() || lines.is_empty() {
        push_line(&mut lines, current, default_height);
    }
    for line in &mut lines {
        reorder_bidi_line(line);
    }
    if lines
        .iter()
        .enumerate()
        .any(|(index, line)| line.width > width - if index == 0 { first_indent } else { 0.0 } + EPS)
    {
        return Err(Error("text line exceeds available width".into()));
    }
    Ok(lines)
}

struct Flow {
    pages: Vec<Page>,
    page_content: bool,
    y: f32,
    page: PageStyle,
    sheet: Sheet,
    page_name: Option<String>,
    fonts: FontRegistry,
    base_dir: PathBuf,
    images: Arc<Mutex<ImageCache>>,
    warnings: Vec<Diagnostic>,
    limits: RenderLimits,
    cancel: Option<Arc<AtomicBool>>,
    experimental_shaping: bool,
    svg_dpi: f32,
    shaping_ns: AtomicU64,
    iterations: usize,
    resource_bytes: usize,
    pending_break: bool,
    pending_break_side: Option<String>,
    frame: Option<(f32, f32)>,
    center_children: bool,
    paint_visible: bool,
}

fn resolved_dimension(fixed: Option<f32>, percent: Option<f32>, reference: f32) -> Option<f32> {
    fixed.or_else(|| percent.map(|value| reference * value))
}

fn constrained_dimension(value: f32, minimum: Option<f32>, maximum: Option<f32>) -> f32 {
    // CSS sizing gives the minimum precedence when min > max.
    value
        .min(maximum.unwrap_or(f32::INFINITY))
        .max(minimum.unwrap_or(0.0))
}

impl Flow {
    fn margin_content(raw: &str, page: usize, pages: usize) -> String {
        let mut out = String::new();
        let mut rest = raw.trim();
        while !rest.is_empty() {
            rest = rest.trim_start();
            if let Some(tail) = rest.strip_prefix("counter(page)") {
                out.push_str(&page.to_string());
                rest = tail;
            } else if let Some(tail) = rest.strip_prefix("counter(pages)") {
                out.push_str(&pages.to_string());
                rest = tail;
            } else if let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') {
                let mut escaped = false;
                let mut end = None;
                for (i, ch) in rest[1..].char_indices() {
                    if escaped {
                        out.push(ch);
                        escaped = false;
                        continue;
                    }
                    if ch == '\\' {
                        escaped = true;
                        continue;
                    }
                    if ch == quote {
                        end = Some(i + 2);
                        break;
                    }
                    out.push(ch);
                }
                if let Some(end) = end {
                    rest = &rest[end..];
                } else {
                    break;
                }
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                if end == 0 {
                    break;
                }
                rest = &rest[end..];
            }
        }
        out
    }
    fn draw_margin_box(
        &mut self,
        index: usize,
        name: &str,
        box_style: &MarginBox,
        total: usize,
        occupied: [bool; 3],
    ) -> Result<()> {
        let content = Self::margin_content(&box_style.content, index + 1, total);
        if content.is_empty() {
            return Ok(());
        }
        let page = &self.pages[index].style;
        let span = page.width - page.margin[1] - page.margin[3];
        let slot = if name.ends_with("left") {
            0
        } else if name.ends_with("right") {
            2
        } else {
            1
        };
        let align = ["left", "center", "right"][slot];
        let count = occupied.iter().filter(|&&value| value).count();
        let (x, w) = if count == 3 || (count == 2 && occupied[1]) {
            // A populated center box needs its own non-overlapping column.
            let w = span / count as f32;
            let rank = occupied[..slot].iter().filter(|&&value| value).count();
            (page.margin[3] + rank as f32 * w, w)
        } else if count == 2 {
            // Keep the wider left zone used by existing left/right footers.
            if slot == 0 {
                (page.margin[3], span * 0.65)
            } else {
                (page.margin[3] + span * 0.75, span * 0.25)
            }
        } else {
            (page.margin[3], span)
        };
        let style = Style {
            family: box_style.family.clone(),
            font_size: box_style.font_size,
            line_height: box_style.font_size * 1.2,
            color: box_style.color,
            ..Style::default()
        };
        let node = Node {
            tag: "#text".into(),
            attrs: HashMap::new(),
            style,
            text: content,
            children: vec![],
        };
        let lines = lines_for(
            &node,
            w,
            0.0,
            &self.fonts,
            self.cancel.as_deref(),
            self.experimental_shaping,
            &self.shaping_ns,
        )?;
        let height: f32 = lines.iter().map(|l| l.height).sum();
        let y = if name.starts_with("@top") {
            (page.margin[0] - height).max(0.0) / 2.0
        } else {
            page.height - page.margin[2] + (page.margin[2] - height).max(0.0) / 2.0
        };
        let mut cursor = y;
        for line in lines {
            let offset = match align {
                "right" => (w - line.width).max(0.0),
                "center" => ((w - line.width) / 2.0).max(0.0),
                _ => 0.0,
            };
            let mut pen = x + offset;
            for glyph in line.glyphs {
                self.pages[index].items.push(Item::Text {
                    ch: glyph.ch,
                    glyph: glyph.id,
                    unicode: glyph.unicode,
                    natural_advance: glyph.natural_advance,
                    font: glyph.font,
                    x: pen + glyph.x_offset,
                    y: cursor + line.baseline - glyph.shift - glyph.y_offset,
                    size: glyph.size,
                    color: glyph.color,
                });
                pen += glyph.advance;
            }
            cursor += line.height;
        }
        Ok(())
    }
    fn render_margin_boxes(&mut self) -> Result<()> {
        let total = self.pages.len();
        for index in 0..total {
            let mut boxes: Vec<_> = self.pages[index].style.boxes.clone().into_iter().collect();
            boxes.sort_by(|left, right| left.0.cmp(&right.0));
            let occupied = |edge: &str| {
                ["left", "center", "right"].map(|side| {
                    boxes.iter().any(|(name, box_style)| {
                        name == &format!("@{edge}-{side}")
                            && !Self::margin_content(&box_style.content, index + 1, total)
                                .is_empty()
                    })
                })
            };
            let top = occupied("top");
            let bottom = occupied("bottom");
            for (name, box_style) in boxes {
                self.draw_margin_box(
                    index,
                    &name,
                    &box_style,
                    total,
                    if name.starts_with("@top") {
                        top
                    } else {
                        bottom
                    },
                )?;
            }
        }
        Ok(())
    }
    fn new(
        sheet: Sheet,
        fonts: FontRegistry,
        base_dir: PathBuf,
        images: Arc<Mutex<ImageCache>>,
        warnings: Vec<Diagnostic>,
        limits: RenderLimits,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Self {
        let mut page = sheet
            .pseudo_pages
            .get(":first")
            .or_else(|| sheet.pseudo_pages.get(":right"))
            .cloned()
            .unwrap_or_else(|| sheet.page.clone());
        if page.landscape_size_keyword {
            page.rotation = 0;
        }
        page.boxes = sheet.margin_boxes_for(None, 0, false);
        Self {
            pages: vec![Page {
                items: vec![],
                style: page.clone(),
                name: None,
            }],
            page_content: false,
            y: page.margin[0],
            page,
            sheet,
            page_name: None,
            fonts,
            base_dir,
            images,
            warnings,
            limits,
            cancel,
            experimental_shaping: false,
            svg_dpi: 144.0,
            shaping_ns: AtomicU64::new(0),
            iterations: 0,
            resource_bytes: 0,
            pending_break: false,
            pending_break_side: None,
            frame: None,
            center_children: false,
            paint_visible: true,
        }
    }
    fn content_width(&self) -> f32 {
        self.frame
            .map(|(_, w)| w)
            .unwrap_or(self.page.width - self.page.margin[1] - self.page.margin[3])
    }
    fn content_x(&self) -> f32 {
        self.frame.map(|(x, _)| x).unwrap_or(self.page.margin[3])
    }
    fn limit(&self) -> f32 {
        self.page.height - self.page.margin[2]
    }
    fn full_height(&self) -> f32 {
        self.limit() - self.page.margin[0]
    }
    fn page_has_content(&self) -> bool {
        self.page_content
    }
    fn new_page(&mut self) -> Result<()> {
        cancelled(self.cancel.as_deref())?;
        if self.pages.len() >= self.limits.max_pages {
            return Err(Error("render limit exceeded: max_pages".into()));
        }
        self.page = self.style_for(self.page_name.as_deref(), self.pages.len(), false);
        self.pages.push(Page {
            items: vec![],
            style: self.page.clone(),
            name: self.page_name.clone(),
        });
        self.page_content = false;
        self.y = self.page.margin[0];
        Ok(())
    }
    fn style_for(&self, name: Option<&str>, index: usize, blank: bool) -> PageStyle {
        let side = if (index + 1).is_multiple_of(2) {
            ":left"
        } else {
            ":right"
        };
        let mut style = name
            .and_then(|n| self.sheet.named_pages.get(n))
            .or_else(|| {
                if blank {
                    self.sheet.pseudo_pages.get(":blank")
                } else {
                    None
                }
            })
            .or_else(|| {
                if index == 0 {
                    self.sheet.pseudo_pages.get(":first")
                } else {
                    None
                }
            })
            .or_else(|| self.sheet.pseudo_pages.get(side))
            .cloned()
            .unwrap_or_else(|| self.sheet.page.clone());
        // `size: landscape` already selects a landscape sheet. In this
        // compatibility case, a second 90-degree PDF page rotation would
        // display the landscape content sideways in common PDF viewers.
        if style.landscape_size_keyword {
            style.rotation = 0;
        }
        style.boxes = self.sheet.margin_boxes_for(name, index, blank);
        style
    }
    fn switch_page(&mut self, name: Option<&str>) -> Result<()> {
        if self.page_name.as_deref() == name {
            return Ok(());
        }
        self.page_name = name.map(str::to_owned);
        if self.page_has_content() {
            self.new_page()?;
        } else {
            self.page = self.style_for(name, self.pages.len() - 1, false);
            let page = self.pages.last_mut().expect("initial page");
            page.style = self.page.clone();
            page.name = self.page_name.clone();
            self.y = self.page.margin[0];
        }
        Ok(())
    }
    fn begin(&mut self, style: &Style) -> Result<()> {
        self.switch_page(style.page_name.as_deref())?;
        let side = if style.break_before {
            style.break_before_side.as_deref()
        } else {
            self.pending_break_side.as_deref()
        }
        .map(str::to_owned);
        if (self.pending_break || style.break_before) && self.page_has_content() {
            self.new_page()?;
        }
        if let Some(side) = side {
            let right = self.pages.len() % 2 == 1;
            if (side == "right") != right {
                let index = self.pages.len() - 1;
                let blank_style = self.style_for(self.page_name.as_deref(), index, true);
                self.pages[index].style = blank_style;
                self.new_page()?;
            }
        }
        self.pending_break = false;
        self.pending_break_side = None;
        Ok(())
    }
    fn finish(&mut self, style: &Style) {
        if style.break_after {
            self.pending_break = true;
            self.pending_break_side = style.break_after_side.clone();
        }
    }
    fn item(&mut self, item: Item) {
        let destination = matches!(item, Item::Destination { .. });
        if !destination {
            self.page_content = true;
        }
        if !self.paint_visible && !destination {
            return;
        }
        if let Some(page) = self.pages.last_mut() {
            page.items.push(item);
        }
    }
    fn visible_item(&mut self, item: Item) {
        if !matches!(item, Item::Destination { .. }) {
            self.page_content = true;
        }
        if let Some(page) = self.pages.last_mut() {
            page.items.push(item);
        }
    }
    fn destination(&mut self, node: &Node, x: f32, y: f32) {
        if let Some(name) = node.attr("id").filter(|name| !name.is_empty()) {
            self.item(Item::Destination {
                name: name.to_owned(),
                x,
                y,
            });
        }
    }
    fn node(&mut self, node: &Node) -> Result<()> {
        cancelled(self.cancel.as_deref())?;
        self.iterations = self.iterations.saturating_add(1);
        if self.iterations > self.limits.max_layout_iterations {
            return Err(Error("render limit exceeded: max_layout_iterations".into()));
        }
        let old_visibility = self.paint_visible;
        self.paint_visible = node.style.visibility == "visible";
        let result = self.positioned_node(node);
        self.paint_visible = old_visibility;
        result
    }
    fn positioned_node(&mut self, node: &Node) -> Result<()> {
        if node.style.position != "relative" {
            return self.node_content(node);
        }
        let item_counts: Vec<usize> = self.pages.iter().map(|p| p.items.len()).collect();
        self.node_content(node)?;
        let dx = node.style.inset[3]
            .or_else(|| node.style.inset[1].map(|right| -right))
            .unwrap_or(0.0);
        let dy = node.style.inset[0]
            .or_else(|| node.style.inset[2].map(|bottom| -bottom))
            .unwrap_or(0.0);
        for (index, page) in self.pages.iter_mut().enumerate() {
            for item in page
                .items
                .iter_mut()
                .skip(*item_counts.get(index).unwrap_or(&0))
            {
                translate_item(item, dx, dy);
            }
        }
        Ok(())
    }
    fn node_content(&mut self, node: &Node) -> Result<()> {
        if node.tag == "#text" {
            if !node.text.trim().is_empty() {
                self.paragraph(node)?;
            }
            return Ok(());
        }
        if node.tag == "table" {
            return self.table(node);
        }
        if node.style.display == "grid" {
            return self.grid(node);
        }
        if node.style.display == "flex" {
            return self.flex(node);
        }
        if node.tag == "img" {
            return self.image(node);
        }
        if matches!(
            node.tag.as_str(),
            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        ) {
            return self.paragraph(node);
        }
        // Container text before/after child blocks is kept in source order.
        self.begin(&node.style)?;
        let mut group = Vec::new();
        for child in &node.children {
            if matches!(
                child.tag.as_str(),
                "#text" | "span" | "strong" | "b" | "em" | "i" | "u" | "a" | "sup" | "sub" | "br"
            ) {
                group.push(child.clone());
            } else {
                if group.iter().any(|c| !c.plain_text().trim().is_empty()) {
                    let mut inline = node.clone();
                    inline.children = std::mem::take(&mut group);
                    inline.style.break_before = false;
                    inline.style.break_after = false;
                    self.paragraph(&inline)?;
                }
                group.clear();
                self.node(child)?;
            }
        }
        if group.iter().any(|c| !c.plain_text().trim().is_empty()) {
            let mut inline = node.clone();
            inline.children = group;
            inline.style.break_before = false;
            inline.style.break_after = false;
            self.paragraph(&inline)?;
        }
        self.finish(&node.style);
        Ok(())
    }
    fn flex(&mut self, node: &Node) -> Result<()> {
        if node.style.flex_direction == "row" {
            return self.grid_with_columns(node, node.children.len().max(1));
        }
        self.begin(&node.style)?;
        let old_frame = self.frame;
        let old_center = self.center_children;
        let x = self.content_x() + node.style.margin[3] + node.style.padding[3];
        let width = self.content_width()
            - node.style.margin[1]
            - node.style.margin[3]
            - node.style.padding[1]
            - node.style.padding[3];
        if width <= 0.0 {
            return Err(Error("flex container has no usable width".into()));
        }
        self.y += node.style.margin[0] + node.style.padding[0];
        let start_y = self.y;
        let start_page = self.pages.len();
        let start_item = self.pages.last().map_or(0, |p| p.items.len());
        self.frame = Some((x, width));
        self.center_children = node.style.align_items == "center";
        for (index, child) in node.children.iter().enumerate() {
            self.node(child)?;
            if index + 1 < node.children.len() {
                self.y += node.style.row_gap;
            }
        }
        self.frame = old_frame;
        self.center_children = old_center;
        if let Some(height) = node
            .style
            .height
            .or_else(|| node.style.height_percent.map(|p| self.full_height() * p))
        {
            if start_page != self.pages.len() {
                return Err(Error(
                    "fixed-height flex container cannot split across pages".into(),
                ));
            }
            let used = self.y - start_y;
            if height > used && node.style.justify_content == "center" {
                let shift = (height - used) / 2.0;
                for item in &mut self.pages.last_mut().expect("page").items[start_item..] {
                    shift_item(item, shift);
                }
            }
            self.y = start_y + height.max(used);
        }
        self.y += node.style.padding[2] + node.style.margin[2];
        self.finish(&node.style);
        Ok(())
    }
    fn grid(&mut self, node: &Node) -> Result<()> {
        self.grid_with_columns(node, node.style.grid_columns.unwrap_or(1))
    }
    fn grid_with_columns(&mut self, node: &Node, columns: usize) -> Result<()> {
        self.begin(&node.style)?;
        let children: Vec<_> = node
            .children
            .iter()
            .filter(|c| c.tag != "#text" || !c.text.trim().is_empty())
            .collect();
        let old_frame = self.frame;
        let outer_x = self.content_x();
        let outer_width = self.content_width();
        let x = outer_x + node.style.margin[3] + node.style.padding[3];
        let width = outer_width
            - node.style.margin[1]
            - node.style.margin[3]
            - node.style.padding[1]
            - node.style.padding[3];
        if width <= 0.0 {
            return Err(Error("grid container has no usable width".into()));
        }
        self.y += node.style.margin[0] + node.style.padding[0];
        let track_width =
            (width - node.style.column_gap * columns.saturating_sub(1) as f32) / columns as f32;
        if track_width <= 0.0 {
            return Err(Error("grid gaps leave no usable track width".into()));
        }
        let row_count = children.len().div_ceil(columns);
        for (row_index, row) in children.chunks(columns).enumerate() {
            if self.limit() - self.y < 40.0 && self.page_has_content() {
                self.new_page()?;
            }
            let row_y = self.y;
            let row_page = self.pages.len();
            let mut cell_ranges = Vec::new();
            let mut row_end = row_y;
            for (col, child) in row.iter().enumerate() {
                self.y = row_y;
                self.frame = Some((
                    x + col as f32 * (track_width + node.style.column_gap),
                    track_width,
                ));
                let start = self.pages.last().map_or(0, |p| p.items.len());
                self.node(child)?;
                if self.pages.len() != row_page {
                    return Err(Error("grid row cannot split across pages".into()));
                }
                cell_ranges.push((start, self.pages.last().expect("page").items.len(), self.y));
                row_end = row_end.max(self.y);
            }
            if node.style.align_items == "center" {
                for (start, end, cell_end) in cell_ranges {
                    let shift = (row_end - cell_end) / 2.0;
                    for item in &mut self.pages.last_mut().expect("page").items[start..end] {
                        shift_item(item, shift);
                    }
                }
            }
            self.y = row_end;
            if row_index + 1 < row_count {
                self.y += node.style.row_gap;
            }
        }
        self.frame = old_frame;
        self.y += node.style.padding[2] + node.style.margin[2];
        self.finish(&node.style);
        Ok(())
    }
    fn paint_box(&mut self, x: f32, y: f32, w: f32, h: f32, style: &Style) {
        if style.visibility == "visible" && (style.background.is_some() || style.border_width > 0.0)
        {
            self.item(Item::Rect {
                x,
                y,
                w,
                h,
                fill: style.background,
                stroke: (style.border_width > 0.0)
                    .then_some((style.border_width, style.border_color)),
            });
        }
    }
    fn draw_line(&mut self, line: &Line, x: f32, y: f32, width: f32, align: &str, last: bool) {
        let has_visible_glyph = line.glyphs.iter().any(|glyph| glyph.visible);
        if has_visible_glyph {
            if let Some(actual) = &line.actual_text {
                self.visible_item(Item::BeginActualText(actual.clone()));
            }
        }
        let offset = match align {
            "center" => (width - line.width) / 2.0,
            "right" => width - line.width,
            _ => 0.0,
        };
        let mut pen = x + offset.max(0.0);
        let spaces = line.glyphs.iter().filter(|g| g.ch == ' ').count();
        let justify_gap = if align == "justify" && !last && spaces > 0 {
            (width - line.width).max(0.0) / spaces as f32
        } else {
            0.0
        };
        for glyph in &line.glyphs {
            let baseline = y + line.baseline - glyph.shift;
            if let Some(inline) = &glyph.inline_box {
                let box_x = pen + inline.style.margin[3];
                let box_y = baseline - inline.height;
                if let Some(name) = &inline.destination {
                    self.visible_item(Item::Destination {
                        name: name.clone(),
                        x: box_x,
                        y: box_y,
                    });
                }
                if inline.style.visibility == "visible"
                    && (inline.style.background.is_some() || inline.style.border_width > 0.0)
                {
                    self.visible_item(Item::Rect {
                        x: box_x,
                        y: box_y,
                        w: inline.width,
                        h: inline.height,
                        fill: inline.style.background,
                        stroke: (inline.style.border_width > 0.0)
                            .then_some((inline.style.border_width, inline.style.border_color)),
                    });
                }
                let content_x = box_x + inline.style.border_width + inline.style.padding[3];
                let mut content_y = box_y + inline.style.border_width + inline.style.padding[0];
                for (line_index, nested) in inline.lines.iter().enumerate() {
                    self.draw_line(
                        nested,
                        content_x,
                        content_y,
                        inline.inner_width,
                        &inline.style.text_align,
                        line_index + 1 == inline.lines.len(),
                    );
                    content_y += nested.height;
                }
                if inline.style.visibility == "visible" {
                    if let Some(target) = &inline.href {
                        self.visible_item(Item::Link {
                            x: box_x,
                            y: box_y,
                            w: inline.width,
                            h: inline.height,
                            target: target.to_string(),
                        });
                    }
                }
                pen += glyph.advance;
                continue;
            }
            if glyph.visible && (!glyph.soft_hyphen || glyph.advance > 0.0) {
                self.visible_item(Item::Text {
                    ch: glyph.ch,
                    glyph: glyph.id,
                    unicode: glyph.unicode.clone(),
                    natural_advance: glyph.natural_advance,
                    font: glyph.font.clone(),
                    x: pen + glyph.x_offset,
                    y: baseline - glyph.y_offset,
                    size: glyph.size,
                    color: glyph.color,
                });
            }
            if glyph.visible
                && (!glyph.soft_hyphen || glyph.advance > 0.0)
                && glyph.decoration != TextDecoration::None
            {
                let decoration_y = if glyph.decoration == TextDecoration::Underline {
                    baseline + glyph.size * 0.08
                } else {
                    baseline - glyph.size * 0.3
                };
                self.visible_item(Item::Rect {
                    x: pen,
                    y: decoration_y,
                    w: glyph.advance.max(0.0),
                    h: (glyph.size * 0.055).max(0.35),
                    fill: Some(glyph.color),
                    stroke: None,
                });
            }
            if glyph.visible && (!glyph.soft_hyphen || glyph.advance > 0.0) {
                if let Some(target) = &glyph.href {
                    self.visible_item(Item::Link {
                        x: pen,
                        y,
                        w: glyph.advance.max(0.0),
                        h: line.height,
                        target: target.to_string(),
                    });
                }
            }
            pen += glyph.advance;
            if glyph.ch == ' ' {
                pen += justify_gap;
            }
        }
        if has_visible_glyph && line.actual_text.is_some() {
            self.visible_item(Item::EndActualText);
        }
    }
    fn paragraph(&mut self, node: &Node) -> Result<()> {
        self.begin(&node.style)?;
        let s = &node.style;
        let containing_width = self.content_width();
        let x = self.content_x() + s.margin[3];
        let horizontal_edges = s.padding[1] + s.padding[3] + 2.0 * s.border_width;
        let available_outer = containing_width - s.margin[1] - s.margin[3];
        let declared = resolved_dimension(s.width, s.width_percent, containing_width);
        let minimum = resolved_dimension(s.min_width, s.min_width_percent, containing_width);
        let maximum = resolved_dimension(s.max_width, s.max_width_percent, containing_width);
        let sizing_width = declared.unwrap_or_else(|| {
            if s.box_sizing == "border-box" {
                available_outer
            } else {
                available_outer - horizontal_edges
            }
        });
        let sizing_width = constrained_dimension(sizing_width, minimum, maximum);
        let w = if s.box_sizing == "border-box" {
            sizing_width
        } else {
            sizing_width + horizontal_edges
        };
        let inner = w - horizontal_edges;
        if inner <= 0.0 {
            return Err(Error("paragraph has no usable width".into()));
        }
        let lines = lines_for(
            node,
            inner,
            s.text_indent,
            &self.fonts,
            self.cancel.as_deref(),
            self.experimental_shaping,
            &self.shaping_ns,
        )?;
        let body_height: f32 = lines.iter().map(|l| l.height).sum();
        let vertical_edges = s.padding[0] + s.padding[2] + 2.0 * s.border_width;
        let natural_height = body_height + vertical_edges;
        let height_reference = self.full_height();
        let declared_height = resolved_dimension(s.height, s.height_percent, height_reference);
        let minimum_height =
            resolved_dimension(s.min_height, s.min_height_percent, height_reference);
        let maximum_height =
            resolved_dimension(s.max_height, s.max_height_percent, height_reference);
        let requested_sizing_height = declared_height.unwrap_or_else(|| {
            if s.box_sizing == "border-box" {
                natural_height
            } else {
                body_height
            }
        });
        let requested_sizing_height =
            constrained_dimension(requested_sizing_height, minimum_height, maximum_height);
        let requested_outer_height = if s.box_sizing == "border-box" {
            requested_sizing_height
        } else {
            requested_sizing_height + vertical_edges
        };
        // Overflow remains visible, so a max-height never discards or overlaps text.
        let box_height = natural_height.max(requested_outer_height);
        if (declared_height.is_some() || minimum_height.is_some())
            && requested_outer_height > self.full_height() + EPS
        {
            return Err(Error("paragraph box exceeds page content height".into()));
        }
        let full_needed = s.margin[0] + box_height + s.margin[2];
        if (s.break_inside_avoid || lines.len() == 1)
            && full_needed <= self.full_height() + EPS
            && self.y + full_needed > self.limit() + EPS
            && self.page_has_content()
        {
            self.new_page()?;
        }
        self.y += s.margin[0];
        let mut offset = 0;
        while offset < lines.len() {
            let start = self.y;
            let overhead = s.padding[0] + s.padding[2] + 2.0 * s.border_width;
            let mut used = overhead;
            let mut end = offset;
            while end < lines.len() && self.y + used + lines[end].height <= self.limit() + EPS {
                used += lines[end].height;
                end += 1;
            }
            if end == offset {
                if self.page_has_content() && self.y > self.page.margin[0] + EPS {
                    self.new_page()?;
                    continue;
                }
                return Err(Error(
                    "text line and padding exceed page content height".into(),
                ));
            }
            if end < lines.len() {
                if offset == 0 && end < s.orphans && self.page_has_content() {
                    self.new_page()?;
                    continue;
                }
                let remaining = lines.len() - end;
                if remaining < s.widows {
                    let move_count = s.widows - remaining;
                    let minimum_here = if offset == 0 { s.orphans } else { 1 };
                    if end - offset >= move_count + minimum_here {
                        end -= move_count;
                        used = overhead
                            + lines[offset..end]
                                .iter()
                                .map(|line| line.height)
                                .sum::<f32>();
                    } else if offset == 0
                        && self.page_has_content()
                        && box_height <= self.full_height() + EPS
                    {
                        self.new_page()?;
                        continue;
                    }
                }
            }
            if offset == 0 && end == lines.len() {
                used = used.max(box_height);
            }
            // Visibility affects paint only; this fragment still occupies the page.
            self.page_content = true;
            if offset == 0 {
                self.destination(node, x, start);
            }
            self.paint_box(x, start, w, used, s);
            let mut line_y = start + s.border_width + s.padding[0];
            for (line_index, line) in lines[offset..end].iter().enumerate() {
                let absolute_index = offset + line_index;
                let first_indent = if absolute_index == 0 {
                    s.text_indent
                } else {
                    0.0
                };
                self.draw_line(
                    line,
                    x + s.border_width + s.padding[3] + first_indent,
                    line_y,
                    inner - first_indent,
                    &s.text_align,
                    absolute_index + 1 == lines.len(),
                );
                line_y += line.height;
            }
            self.y += used;
            offset = end;
            if offset < lines.len() {
                self.new_page()?;
            }
        }
        self.y += s.margin[2];
        self.finish(s);
        Ok(())
    }
    fn image(&mut self, node: &Node) -> Result<()> {
        self.begin(&node.style)?;
        let src = node
            .attr("src")
            .ok_or_else(|| Error("img needs src".into()))?;
        let (bytes, svg) = if src.starts_with("data:") {
            let (bytes, format) = data_image(
                src,
                self.limits
                    .max_resource_bytes
                    .saturating_sub(self.resource_bytes),
            )?;
            (bytes, matches!(format, ImageFormat::Svg))
        } else {
            if src.contains("://") {
                return Err(Error("only local files and data URIs are supported".into()));
            }
            let path = local_resource_path(&self.base_dir, src, self.limits.restrict_base_dir)?;
            let file_size = std::fs::metadata(&path)
                .map_err(|e| Error(format!("image {}: {e}", path.display())))?
                .len() as usize;
            if self.resource_bytes.saturating_add(file_size) > self.limits.max_resource_bytes {
                return Err(Error("render limit exceeded: max_resource_bytes".into()));
            }
            let bytes = std::fs::read(&path)
                .map_err(|e| Error(format!("image {}: {e}", path.display())))?;
            let svg = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
            (bytes, svg)
        };
        self.resource_bytes = self.resource_bytes.saturating_add(bytes.len());
        if self.resource_bytes > self.limits.max_resource_bytes {
            return Err(Error("render limit exceeded: max_resource_bytes".into()));
        }
        let dimensions = if svg {
            None
        } else {
            let dimensions = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|e| Error(format!("image format: {e}")))?
                .into_dimensions()
                .map_err(|e| Error(format!("image dimensions: {e}")))?;
            if u64::from(dimensions.0) * u64::from(dimensions.1) > self.limits.max_image_pixels {
                return Err(Error("render limit exceeded: max_image_pixels".into()));
            }
            Some(dimensions)
        };
        cancelled(self.cancel.as_deref())?;
        let digest = if svg {
            let mut hash = Sha256::new();
            hash.update(&bytes);
            hash.update(self.svg_dpi.to_bits().to_be_bytes());
            hash.finalize().into()
        } else {
            Sha256::digest(&bytes).into()
        };
        let hit = {
            let mut cache = self.images.lock().map_err(|e| Error(e.to_string()))?;
            let hit = cache.entries.get(&digest).cloned();
            if hit.is_some() {
                cache.hits += 1;
            } else {
                cache.misses += 1;
            }
            hit
        };
        let data = if let Some(hit) = hit {
            hit
        } else {
            let data = if svg {
                Arc::new(render_svg(
                    &bytes,
                    digest,
                    self.svg_dpi,
                    self.limits.max_image_pixels,
                    &self.fonts,
                )?)
            } else {
                let dimensions = dimensions.expect("raster dimensions");
                let jpeg_metadata =
                    if image::guess_format(&bytes).ok() == Some(image::ImageFormat::Jpeg) {
                        let mut decoder =
                            image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&bytes))
                                .map_err(|e| Error(format!("JPEG metadata: {e}")))?;
                        Some((
                            decoder
                                .orientation()
                                .map_err(|e| Error(format!("JPEG orientation: {e}")))?,
                            decoder.color_type(),
                        ))
                    } else {
                        None
                    };
                let data = if jpeg_metadata
                    == Some((
                        image::metadata::Orientation::NoTransforms,
                        image::ColorType::Rgb8,
                    )) {
                    Arc::new(ImageData {
                        rgb: Vec::new(),
                        alpha: None,
                        jpeg: Some(bytes),
                        width: dimensions.0,
                        height: dimensions.1,
                        display_width: dimensions.0 as f32 * 0.75,
                        display_height: dimensions.1 as f32 * 0.75,
                        digest,
                    })
                } else {
                    let mut image = image::load_from_memory(&bytes)
                        .map_err(|e| Error(format!("image decode: {e}")))?;
                    if let Some((orientation, _)) = jpeg_metadata {
                        image.apply_orientation(orientation);
                    }
                    let (iw, ih) = image.dimensions();
                    let rgba = image.to_rgba8();
                    let mut rgb = Vec::with_capacity((iw as usize) * (ih as usize) * 3);
                    let mut alpha = Vec::with_capacity((iw as usize) * (ih as usize));
                    for pixel in rgba.pixels() {
                        rgb.extend_from_slice(&pixel.0[..3]);
                        alpha.push(pixel[3]);
                    }
                    let alpha = alpha.iter().any(|value| *value != 255).then_some(alpha);
                    Arc::new(ImageData {
                        rgb,
                        alpha,
                        jpeg: None,
                        width: iw,
                        height: ih,
                        display_width: iw as f32 * 0.75,
                        display_height: ih as f32 * 0.75,
                        digest,
                    })
                };
                data
            };
            let mut cache = self.images.lock().map_err(|e| Error(e.to_string()))?;
            let size = data.retained_bytes();
            if size <= IMAGE_CACHE_LIMIT {
                while cache.bytes + size > IMAGE_CACHE_LIMIT {
                    let Some(old) = cache.order.pop_front() else {
                        break;
                    };
                    if let Some(item) = cache.entries.remove(&old) {
                        cache.bytes -= item.retained_bytes();
                    }
                }
                cache.bytes += size;
                cache.order.push_back(digest);
                cache.entries.insert(digest, data.clone());
            }
            data
        };
        let (iw, ih) = (data.display_width, data.display_height);
        let mut width = node
            .style
            .width
            .or_else(|| node.style.width_percent.map(|p| self.content_width() * p))
            .unwrap_or(iw)
            .min(self.content_width());
        width = constrained_dimension(
            width,
            resolved_dimension(
                node.style.min_width,
                node.style.min_width_percent,
                self.content_width(),
            ),
            resolved_dimension(
                node.style.max_width,
                node.style.max_width_percent,
                self.content_width(),
            ),
        );
        let mut height = node
            .style
            .height
            .or_else(|| node.style.height_percent.map(|p| self.full_height() * p))
            .unwrap_or_else(|| width / node.style.aspect_ratio.unwrap_or(iw / ih));
        if let Some(max_height) = resolved_dimension(
            node.style.max_height,
            node.style.max_height_percent,
            self.full_height(),
        ) {
            if height > max_height {
                let scale = max_height / height;
                height = max_height;
                width *= scale;
            }
        }
        if let Some(min_height) = resolved_dimension(
            node.style.min_height,
            node.style.min_height_percent,
            self.full_height(),
        ) {
            if height < min_height {
                let scale = min_height / height;
                height = min_height;
                width *= scale;
            }
        }
        if height > self.full_height() + EPS {
            return Err(Error("image exceeds page content height".into()));
        }
        if self.y + height > self.limit() + EPS && self.page_has_content() {
            self.new_page()?;
        }
        self.destination(node, self.content_x(), self.y);
        self.item(Item::Image {
            data,
            x: self.content_x()
                + if self.center_children {
                    (self.content_width() - width).max(0.0) / 2.0
                } else {
                    0.0
                },
            y: self.y,
            w: width,
            h: height,
        });
        self.y += height;
        self.finish(&node.style);
        Ok(())
    }
}

#[derive(Clone)]
struct Cell {
    style: Style,
    lines: Vec<Line>,
    items: Vec<Item>,
    content_height: f32,
    column: usize,
    colspan: usize,
    rowspan: usize,
    borders: [TableBorder; 4],
    draw_borders: [bool; 4],
}
#[derive(Clone, Copy)]
struct TableBorder {
    width: f32,
    color: Color,
}
impl TableBorder {
    fn from_style(style: &Style) -> Self {
        Self {
            width: style.border_width,
            color: style.border_color,
        }
    }
    fn winner(self, other: Self) -> Self {
        if other.width + EPS >= self.width {
            other
        } else {
            self
        }
    }
}
#[derive(Clone)]
struct Row {
    cells: Vec<Cell>,
    line_count: usize,
    step: f32,
    pad: f32,
    x: f32,
    column_gap: f32,
    row_gap: f32,
    collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableSection {
    Head,
    Body,
    Foot,
}

#[derive(Clone)]
struct PlacedCell {
    node: Node,
    column: usize,
    colspan: usize,
    rowspan: usize,
}

#[derive(Clone)]
struct PlacedRow {
    cells: Vec<PlacedCell>,
    section: TableSection,
}
impl Flow {
    fn table_span(cell: &Node, attribute: &str) -> Result<usize> {
        match cell.attr(attribute) {
            None => Ok(1),
            Some(raw) => raw
                .parse::<usize>()
                .ok()
                .filter(|span| (1..=1024).contains(span))
                .ok_or_else(|| Error(format!("invalid table {attribute}: {raw}"))),
        }
    }
    fn colspan(cell: &Node) -> Result<usize> {
        Self::table_span(cell, "colspan")
    }
    fn rowspan(cell: &Node) -> Result<usize> {
        Self::table_span(cell, "rowspan")
    }
    fn rows(
        node: &Node,
        section: TableSection,
        group: usize,
        next_group: &mut usize,
        out: &mut Vec<(Node, TableSection, usize)>,
    ) {
        for child in &node.children {
            if child.tag == "tr" {
                out.push((child.clone(), section, group));
            } else if matches!(child.tag.as_str(), "thead" | "tbody" | "tfoot") {
                let child_section = match child.tag.as_str() {
                    "thead" if child.style.display == "table-header-group" => TableSection::Head,
                    "tfoot" if child.style.display == "table-footer-group" => TableSection::Foot,
                    _ => TableSection::Body,
                };
                *next_group += 1;
                Self::rows(child, child_section, *next_group, next_group, out);
            }
        }
    }
    fn columns(node: &Node, out: &mut Vec<Style>) -> Result<()> {
        for child in &node.children {
            if child.tag == "col" {
                let span = child
                    .attr("span")
                    .unwrap_or("1")
                    .parse::<usize>()
                    .ok()
                    .filter(|span| (1..=1024).contains(span))
                    .ok_or_else(|| Error("invalid table column span".into()))?;
                out.extend(std::iter::repeat_n(child.style.clone(), span));
            } else if child.tag == "colgroup" {
                if child.children.iter().any(|column| column.tag == "col") {
                    Self::columns(child, out)?;
                } else {
                    let span = child
                        .attr("span")
                        .unwrap_or("1")
                        .parse::<usize>()
                        .ok()
                        .filter(|span| (1..=1024).contains(span))
                        .ok_or_else(|| Error("invalid table column-group span".into()))?;
                    out.extend(std::iter::repeat_n(child.style.clone(), span));
                }
            }
        }
        Ok(())
    }
    fn place_rows(row_nodes: Vec<(Node, TableSection, usize)>) -> Result<(Vec<PlacedRow>, usize)> {
        let mut rows = Vec::with_capacity(row_nodes.len());
        let mut occupied = Vec::<usize>::new();
        let mut current_group = None;
        let mut columns = 0usize;
        for (row, section, group) in row_nodes {
            if current_group != Some(group) {
                if occupied.iter().any(|remaining| *remaining > 0) {
                    return Err(Error("table rowspan crosses a row-group boundary".into()));
                }
                occupied.clear();
                current_group = Some(group);
            }
            let mut cells = Vec::new();
            let mut search_from = 0usize;
            for cell in row
                .children
                .iter()
                .filter(|cell| matches!(cell.tag.as_str(), "td" | "th"))
            {
                let colspan = Self::colspan(cell)?;
                let rowspan = Self::rowspan(cell)?;
                let mut column = search_from;
                loop {
                    let end = column
                        .checked_add(colspan)
                        .filter(|end| *end <= 1024)
                        .ok_or_else(|| Error("table has too many columns".into()))?;
                    if occupied.len() < end {
                        occupied.resize(end, 0);
                    }
                    if occupied[column..end]
                        .iter()
                        .all(|remaining| *remaining == 0)
                    {
                        break;
                    }
                    column += 1;
                }
                let end = column + colspan;
                for slot in &mut occupied[column..end] {
                    *slot = rowspan;
                }
                cells.push(PlacedCell {
                    node: cell.clone(),
                    column,
                    colspan,
                    rowspan,
                });
                search_from = end;
                columns = columns.max(end);
            }
            rows.push(PlacedRow { cells, section });
            for remaining in &mut occupied {
                *remaining = remaining.saturating_sub(1);
            }
        }
        if occupied.iter().any(|remaining| *remaining > 0) {
            return Err(Error("table rowspan exceeds its row group".into()));
        }
        Ok((rows, columns))
    }
    fn table(&mut self, table: &Node) -> Result<()> {
        self.begin(&table.style)?;
        self.y += table.style.margin[0];
        let mut row_nodes = Vec::new();
        let mut next_group = 0usize;
        Self::rows(
            table,
            TableSection::Body,
            0,
            &mut next_group,
            &mut row_nodes,
        );
        if row_nodes.is_empty() {
            self.finish(&table.style);
            return Ok(());
        }
        let (row_nodes, columns) = Self::place_rows(row_nodes)?;
        if columns == 0 {
            self.finish(&table.style);
            return Ok(());
        }
        let containing_width = self.content_width();
        let available_width = containing_width - table.style.margin[1] - table.style.margin[3];
        let declared_width = resolved_dimension(
            table.style.width,
            table.style.width_percent,
            containing_width,
        )
        .unwrap_or(available_width);
        let minimum_width = resolved_dimension(
            table.style.min_width,
            table.style.min_width_percent,
            containing_width,
        );
        let maximum_width = resolved_dimension(
            table.style.max_width,
            table.style.max_width_percent,
            containing_width,
        );
        let width = constrained_dimension(declared_width, minimum_width, maximum_width);
        if width <= 0.0 || width > available_width + EPS {
            return Err(Error(format!(
                "table width {width:.3} exceeds available width {available_width:.3}"
            )));
        }
        let table_x = self.content_x() + table.style.margin[3];
        let spacing = if table.style.border_collapse == "separate" {
            table.style.border_spacing
        } else {
            [0.0; 2]
        };
        let track_width = width - spacing[0] * columns.saturating_sub(1) as f32;
        if track_width <= 0.0 {
            return Err(Error("table border spacing leaves no usable width".into()));
        }
        let mut column_styles = Vec::new();
        Self::columns(table, &mut column_styles)?;
        let widths = self.column_widths(
            &row_nodes,
            &column_styles,
            columns,
            track_width,
            table.style.table_layout == "fixed",
        )?;
        let mut heads = Vec::new();
        let mut bodies = Vec::new();
        let mut foots = Vec::new();
        let mut layouts = Vec::new();
        for row in row_nodes {
            let layout = self.layout_row(
                &row,
                &widths,
                spacing,
                table_x,
                table.style.border_collapse == "collapse",
            )?;
            layouts.push((row.section, layout));
        }
        for (section, layout) in layouts {
            match section {
                TableSection::Head => heads.push(layout),
                TableSection::Body => bodies.push(layout),
                TableSection::Foot => foots.push(layout),
            }
        }
        if !heads.is_empty() && bodies.is_empty() {
            return Err(Error("table header has no data rows".into()));
        }
        if bodies.is_empty() {
            bodies.append(&mut foots);
        }
        if table.style.border_collapse == "collapse" {
            let head_count = heads.len();
            let body_count = bodies.len();
            let mut rows = heads
                .iter()
                .chain(&bodies)
                .chain(&foots)
                .cloned()
                .collect::<Vec<_>>();
            Self::resolve_collapsed_borders(&mut rows, columns);
            heads = rows.drain(..head_count).collect();
            bodies = rows.drain(..body_count).collect();
            foots = rows;
        }
        let natural_height = Self::table_section_height(&heads)
            + Self::table_section_height(&bodies)
            + Self::table_section_height(&foots);
        let height_reference = self.full_height();
        let declared_height = resolved_dimension(
            table.style.height,
            table.style.height_percent,
            height_reference,
        );
        let minimum_height = resolved_dimension(
            table.style.min_height,
            table.style.min_height_percent,
            height_reference,
        );
        let maximum_height = resolved_dimension(
            table.style.max_height,
            table.style.max_height_percent,
            height_reference,
        );
        let target_height = constrained_dimension(
            declared_height.unwrap_or(natural_height),
            minimum_height,
            maximum_height,
        )
        .max(natural_height);
        if target_height > natural_height + EPS {
            let row_count = heads.len() + bodies.len() + foots.len();
            let extra = (target_height - natural_height) / row_count.max(1) as f32;
            for row in heads.iter_mut().chain(&mut bodies).chain(&mut foots) {
                row.pad += extra;
            }
        }
        let header_height = Self::table_section_height(&heads);
        let footer_height = Self::table_section_height(&foots);
        if header_height + footer_height >= self.full_height() - EPS {
            return Err(Error(
                "repeating table header and footer fill page content area".into(),
            ));
        }
        let total_height = header_height + footer_height + Self::table_section_height(&bodies);
        if table.style.break_inside_avoid
            && total_height <= self.full_height() + EPS
            && self.y + total_height > self.limit() + EPS
            && self.page_has_content()
        {
            self.new_page()?;
        }
        let mut header_drawn = false;
        let mut row_index = 0usize;
        while row_index < bodies.len() {
            let group_end = Self::rowspan_group_end(&bodies, row_index);
            if group_end > row_index + 1 {
                let heights = Self::rowspan_group_heights(&bodies, row_index, group_end);
                let group_height: f32 = heights.iter().sum();
                let usable = self.full_height() - header_height - footer_height;
                if !header_drawn {
                    let required = group_height.min(usable);
                    if self.y + header_height + required + footer_height > self.limit() + EPS
                        && self.page_has_content()
                    {
                        self.new_page()?;
                    }
                    self.draw_table_section(&heads, &widths);
                    header_drawn = true;
                } else if group_height <= usable + EPS
                    && self.y + group_height > self.limit() - footer_height + EPS
                {
                    self.draw_table_footer(&foots, &widths);
                    self.new_page()?;
                    self.draw_table_section(&heads, &widths);
                }
                if group_height <= usable + EPS {
                    self.draw_rowspan_group(&bodies, &widths, row_index, group_end, &heights);
                } else {
                    let mut fragment_start = 0.0;
                    while fragment_start < group_height - EPS {
                        let available = self.limit() - footer_height - self.y;
                        if available <= EPS {
                            self.draw_table_footer(&foots, &widths);
                            self.new_page()?;
                            self.draw_table_section(&heads, &widths);
                            continue;
                        }
                        let desired_end = (fragment_start + available).min(group_height);
                        let fragment_end = Self::rowspan_fragment_end(
                            &bodies,
                            row_index,
                            group_end,
                            &heights,
                            fragment_start,
                            desired_end,
                        )?;
                        self.draw_rowspan_fragment(
                            &bodies,
                            &widths,
                            row_index,
                            group_end,
                            &heights,
                            fragment_start..fragment_end,
                        );
                        fragment_start = fragment_end;
                        if fragment_start < group_height - EPS {
                            self.draw_table_footer(&foots, &widths);
                            self.new_page()?;
                            self.draw_table_section(&heads, &widths);
                        }
                    }
                }
                row_index = group_end;
                continue;
            }
            let row = &bodies[row_index];
            if !header_drawn {
                self.ensure_header(&heads, &widths, header_height, footer_height, row)?;
                header_drawn = true;
            }
            let mut offset = 0;
            while offset < row.line_count {
                let remaining = row.line_count - offset;
                let row_height = remaining as f32 * row.step + row.pad + row.row_gap;
                if row_height <= self.full_height() - header_height - footer_height + EPS
                    && self.y + row_height > self.limit() - footer_height + EPS
                    && self.page_has_content()
                {
                    self.draw_table_footer(&foots, &widths);
                    self.new_page()?;
                    self.ensure_header(&heads, &widths, header_height, footer_height, row)?;
                }
                let available = self.limit() - footer_height - self.y - row.pad - row.row_gap;
                let take = ((available + EPS) / row.step).floor().max(0.0) as usize;
                if take == 0 {
                    if self.y > self.page.margin[0] + header_height + EPS {
                        self.draw_table_footer(&foots, &widths);
                        self.new_page()?;
                        self.ensure_header(&heads, &widths, header_height, footer_height, row)?;
                        continue;
                    }
                    return Err(Error(
                        "table row cannot place one text line after header and footer".into(),
                    ));
                }
                let take = take.min(remaining);
                self.draw_row(row, &widths, offset, take);
                offset += take;
                if offset < row.line_count {
                    self.draw_table_footer(&foots, &widths);
                    self.new_page()?;
                    self.ensure_header(&heads, &widths, header_height, footer_height, row)?;
                }
            }
            row_index += 1;
        }
        self.draw_table_footer(&foots, &widths);
        self.y += table.style.margin[2];
        self.finish(&table.style);
        Ok(())
    }
    // Explicit cell widths are honored first; remaining space follows content minimums.
    fn column_widths(
        &self,
        rows: &[PlacedRow],
        column_styles: &[Style],
        columns: usize,
        total: f32,
        fixed: bool,
    ) -> Result<Vec<f32>> {
        let mut desired = vec![if fixed { 0.0f32 } else { 20.0f32 }; columns];
        let mut minimums = vec![1.0f32; columns];
        let mut maximums = vec![f32::INFINITY; columns];
        let mut explicit = vec![false; columns];
        for (index, style) in column_styles.iter().take(columns).enumerate() {
            let minimum =
                resolved_dimension(style.min_width, style.min_width_percent, total).unwrap_or(1.0);
            let maximum = resolved_dimension(style.max_width, style.max_width_percent, total)
                .unwrap_or(f32::INFINITY)
                .max(minimum);
            minimums[index] = minimum;
            maximums[index] = maximum;
            if let Some(width) = resolved_dimension(style.width, style.width_percent, total) {
                desired[index] = width.clamp(minimum, maximum);
                explicit[index] = true;
            }
        }
        for row in rows.iter().take(if fixed { 1 } else { rows.len() }) {
            for placed in &row.cells {
                let cell = &placed.node;
                let column = placed.column;
                let span = placed.colspan;
                if column + span > columns {
                    return Err(Error("table colspan exceeds column count".into()));
                }
                let cell_min =
                    resolved_dimension(cell.style.min_width, cell.style.min_width_percent, total)
                        .unwrap_or(0.0);
                let cell_max =
                    resolved_dimension(cell.style.max_width, cell.style.max_width_percent, total)
                        .unwrap_or(f32::INFINITY)
                        .max(cell_min);
                for i in column..column + span {
                    minimums[i] = minimums[i].max(cell_min / span as f32);
                    maximums[i] = maximums[i].min(cell_max / span as f32);
                    maximums[i] = maximums[i].max(minimums[i]);
                }
                if let Some(w) =
                    resolved_dimension(cell.style.width, cell.style.width_percent, total)
                {
                    for i in column..column + span {
                        desired[i] =
                            desired[i].max((w / span as f32).clamp(minimums[i], maximums[i]));
                        explicit[i] = true;
                    }
                } else if !fixed {
                    let text = cell.plain_text();
                    let longest = if matches!(cell.style.white_space.as_str(), "nowrap" | "pre") {
                        text.lines()
                            .map(|line| line.chars().count())
                            .max()
                            .unwrap_or(0)
                    } else if cell.style.overflow_wrap == "anywhere"
                        || cell.style.word_break == "break-all"
                    {
                        usize::from(!text.is_empty())
                    } else {
                        text.split(|character: char| {
                            character.is_whitespace() || character == '\u{00ad}'
                        })
                        .map(|word| word.chars().count())
                        .max()
                        .unwrap_or(0)
                    }
                    .min(80);
                    let minimum = (longest as f32 * cell.style.font_size * 0.55
                        + cell.style.padding[1]
                        + cell.style.padding[3]
                        + 2.0 * cell.style.border_width)
                        / span as f32;
                    for i in column..column + span {
                        if !explicit[i] {
                            desired[i] = desired[i].max(minimum).clamp(minimums[i], maximums[i]);
                        }
                    }
                }
            }
        }
        if fixed {
            let specified: f32 = desired.iter().sum();
            let unspecified = explicit.iter().filter(|assigned| !**assigned).count();
            if unspecified > 0 {
                let share = ((total - specified) / unspecified as f32).max(1.0);
                for (width, assigned) in desired.iter_mut().zip(&explicit) {
                    if !assigned {
                        *width = share;
                    }
                }
            } else if specified < total {
                let extra = (total - specified) / columns as f32;
                for width in &mut desired {
                    *width += extra;
                }
            }
        }
        for index in 0..columns {
            desired[index] = desired[index].clamp(minimums[index], maximums[index]);
        }
        let minimum_sum: f32 = minimums.iter().sum();
        if minimum_sum > total + EPS {
            return Err(Error(
                "table minimum column widths exceed available width".into(),
            ));
        }
        for _ in 0..columns.saturating_mul(2).max(1) {
            let sum: f32 = desired.iter().sum();
            let delta = total - sum;
            if delta.abs() <= EPS {
                break;
            }
            let adjustable = (0..columns)
                .filter(|index| {
                    if delta > 0.0 {
                        desired[*index] + EPS < maximums[*index]
                    } else {
                        desired[*index] > minimums[*index] + EPS
                    }
                })
                .collect::<Vec<_>>();
            if adjustable.is_empty() {
                break;
            }
            let share = delta / adjustable.len() as f32;
            for index in adjustable {
                desired[index] = (desired[index] + share).clamp(minimums[index], maximums[index]);
            }
        }
        let sum: f32 = desired.iter().sum();
        if sum <= 0.0 || (sum - total).abs() > 0.1 {
            return Err(Error("table width is zero".into()));
        }
        Ok(desired)
    }
    fn contains_nested_table(node: &Node) -> bool {
        node.children
            .iter()
            .any(|child| child.tag == "table" || Self::contains_nested_table(child))
    }
    fn layout_nested_cell(&mut self, cell: &Node, width: f32) -> Result<(Vec<Item>, f32)> {
        let mut local = Flow::new(
            self.sheet.clone(),
            self.fonts.clone(),
            self.base_dir.clone(),
            self.images.clone(),
            vec![],
            self.limits.clone(),
            self.cancel.clone(),
        );
        let local_page = PageStyle {
            width,
            height: 1_000_000.0,
            margin: [0.0; 4],
            ..PageStyle::default()
        };
        local.page = local_page.clone();
        local.pages = vec![Page {
            items: vec![],
            style: local_page,
            name: None,
        }];
        local.y = 0.0;
        local.frame = Some((0.0, width));
        local.experimental_shaping = self.experimental_shaping;
        local.svg_dpi = self.svg_dpi;
        local.resource_bytes = self.resource_bytes;
        let mut container = cell.clone();
        container.tag = "div".into();
        container.attrs.clear();
        container.style.margin = [0.0; 4];
        container.style.padding = [0.0; 4];
        container.style.border_width = 0.0;
        container.style.background = None;
        container.style.width = None;
        container.style.width_percent = None;
        container.style.min_width = None;
        container.style.min_width_percent = None;
        container.style.max_width = None;
        container.style.max_width_percent = None;
        container.style.height = None;
        container.style.height_percent = None;
        container.style.min_height = None;
        container.style.min_height_percent = None;
        container.style.max_height = None;
        container.style.max_height_percent = None;
        container.style.break_before = false;
        container.style.break_after = false;
        local.node_content(&container)?;
        if local.pages.len() != 1 {
            return Err(Error(
                "nested table exceeds the containing table cell".into(),
            ));
        }
        self.resource_bytes = local.resource_bytes;
        self.iterations = self.iterations.saturating_add(local.iterations);
        self.shaping_ns
            .fetch_add(local.shaping_ns.load(Ordering::Relaxed), Ordering::Relaxed);
        self.warnings.extend(local.warnings);
        Ok((local.pages.remove(0).items, local.y))
    }
    fn layout_row(
        &mut self,
        row: &PlacedRow,
        widths: &[f32],
        spacing: [f32; 2],
        x: f32,
        collapsed: bool,
    ) -> Result<Row> {
        let mut cells = Vec::new();
        let mut count = 1;
        let mut step = 0.0f32;
        let mut pad = 0.0f32;
        for placed in &row.cells {
            let cell = &placed.node;
            let column = placed.column;
            let colspan = placed.colspan;
            let width: f32 = widths[column..column + colspan].iter().sum::<f32>()
                + spacing[0] * colspan.saturating_sub(1) as f32;
            let s = &cell.style;
            let inner = width - s.padding[1] - s.padding[3] - 2.0 * s.border_width;
            if inner <= 0.0 {
                return Err(Error("table cell has no usable width".into()));
            }
            let (lines, items, content_height) = if Self::contains_nested_table(cell) {
                let (items, height) = self.layout_nested_cell(cell, inner)?;
                (vec![], items, height)
            } else {
                let lines = lines_for(
                    cell,
                    inner,
                    0.0,
                    &self.fonts,
                    self.cancel.as_deref(),
                    self.experimental_shaping,
                    &self.shaping_ns,
                )?;
                let height = lines.iter().map(|line| line.height).sum();
                (lines, vec![], height)
            };
            let cell_step = if lines.is_empty() {
                content_height.max(s.line_height)
            } else {
                lines
                    .iter()
                    .map(|line| line.height)
                    .fold(s.line_height, f32::max)
            };
            step = step.max(cell_step);
            if placed.rowspan == 1 {
                count = count.max(lines.len().max(1));
                let edges = s.padding[0] + s.padding[2] + 2.0 * s.border_width;
                let natural = content_height + edges;
                let requested = resolved_dimension(s.height, s.height_percent, self.full_height())
                    .unwrap_or(natural);
                let requested = constrained_dimension(
                    requested,
                    resolved_dimension(s.min_height, s.min_height_percent, self.full_height()),
                    resolved_dimension(s.max_height, s.max_height_percent, self.full_height()),
                )
                .max(natural);
                pad = pad.max(edges + (requested - natural));
            }
            let border = TableBorder::from_style(s);
            cells.push(Cell {
                style: s.clone(),
                lines,
                items,
                content_height,
                column,
                colspan,
                rowspan: placed.rowspan,
                borders: [border; 4],
                draw_borders: [true; 4],
            });
        }
        if step <= 0.0 {
            return Err(Error("table row line height is zero".into()));
        }
        Ok(Row {
            cells,
            line_count: count,
            step,
            pad,
            x,
            column_gap: spacing[0],
            row_gap: spacing[1],
            collapsed,
        })
    }
    fn table_row_height(row: &Row) -> f32 {
        row.line_count as f32 * row.step + row.pad + row.row_gap
    }
    fn table_section_height(rows: &[Row]) -> f32 {
        let mut height = 0.0;
        let mut index = 0;
        while index < rows.len() {
            let end = Self::rowspan_group_end(rows, index);
            if end > index + 1 {
                height += Self::rowspan_group_heights(rows, index, end)
                    .iter()
                    .sum::<f32>();
            } else {
                height += Self::table_row_height(&rows[index]);
            }
            index = end;
        }
        height
    }
    fn resolve_collapsed_borders(rows: &mut [Row], columns: usize) {
        if rows.is_empty() {
            return;
        }
        let mut owners = vec![vec![None; columns]; rows.len()];
        for (row_index, row) in rows.iter().enumerate() {
            for (cell_index, cell) in row.cells.iter().enumerate() {
                for slots in owners
                    .iter_mut()
                    .take((row_index + cell.rowspan).min(rows.len()))
                    .skip(row_index)
                {
                    for slot in slots
                        .iter_mut()
                        .take((cell.column + cell.colspan).min(columns))
                        .skip(cell.column)
                    {
                        *slot = Some((row_index, cell_index));
                    }
                }
            }
        }
        let borders = rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| TableBorder::from_style(&cell.style))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let row_count = rows.len();
        for (row_index, row) in rows.iter_mut().enumerate() {
            for (cell_index, cell) in row.cells.iter_mut().enumerate() {
                cell.draw_borders = [false; 4];
                let own = borders[row_index][cell_index];
                let row_end = (row_index + cell.rowspan).min(owners.len());
                let column_end = (cell.column + cell.colspan).min(columns);
                let mut top = own;
                if row_index == 0 {
                    cell.draw_borders[0] = true;
                } else {
                    for owner in &owners[row_index - 1][cell.column..column_end] {
                        if let Some((other_row, other_cell)) = *owner {
                            if (other_row, other_cell) != (row_index, cell_index) {
                                top = borders[other_row][other_cell].winner(top);
                                cell.draw_borders[0] = true;
                            }
                        } else {
                            cell.draw_borders[0] = true;
                        }
                    }
                }
                cell.borders[0] = top;
                let mut left = own;
                if cell.column == 0 {
                    cell.draw_borders[3] = true;
                } else {
                    for slots in owners.iter().take(row_end).skip(row_index) {
                        if let Some((other_row, other_cell)) = slots[cell.column - 1] {
                            if (other_row, other_cell) != (row_index, cell_index) {
                                left = borders[other_row][other_cell].winner(left);
                                cell.draw_borders[3] = true;
                            }
                        } else {
                            cell.draw_borders[3] = true;
                        }
                    }
                }
                cell.borders[3] = left;
                if column_end == columns {
                    cell.draw_borders[1] = true;
                }
                if row_end == row_count {
                    cell.draw_borders[2] = true;
                }
            }
        }
    }
    fn paint_table_cell(
        &mut self,
        cell: &Cell,
        rect: (f32, f32, f32, f32),
        fragment_edges: (bool, bool),
        collapsed: bool,
    ) {
        let (x, y, width, height) = rect;
        let (first_fragment, last_fragment) = fragment_edges;
        if !collapsed {
            self.paint_box(x, y, width, height, &cell.style);
            return;
        }
        if cell.style.visibility != "visible" {
            return;
        }
        if let Some(fill) = cell.style.background {
            self.item(Item::Rect {
                x,
                y,
                w: width,
                h: height,
                fill: Some(fill),
                stroke: None,
            });
        }
        let mut edge = |side: usize, ex: f32, ey: f32, ew: f32, eh: f32| {
            let border = cell.borders[side];
            if cell.draw_borders[side] && border.width > 0.0 {
                self.item(Item::Rect {
                    x: ex,
                    y: ey,
                    w: ew.max(border.width),
                    h: eh.max(border.width),
                    fill: Some(border.color),
                    stroke: None,
                });
            }
        };
        if first_fragment {
            edge(0, x, y, width, cell.borders[0].width);
        }
        edge(
            1,
            x + width - cell.borders[1].width,
            y,
            cell.borders[1].width,
            height,
        );
        if last_fragment {
            edge(
                2,
                x,
                y + height - cell.borders[2].width,
                width,
                cell.borders[2].width,
            );
        }
        edge(3, x, y, cell.borders[3].width, height);
    }
    fn draw_cell_items(&mut self, cell: &Cell, x: f32, y: f32) {
        for original in &cell.items {
            let mut item = original.clone();
            translate_item(&mut item, x, y);
            self.item(item);
        }
    }
    fn draw_table_section(&mut self, rows: &[Row], widths: &[f32]) {
        let mut index = 0;
        while index < rows.len() {
            let end = Self::rowspan_group_end(rows, index);
            if end > index + 1 {
                let heights = Self::rowspan_group_heights(rows, index, end);
                self.draw_rowspan_group(rows, widths, index, end, &heights);
            } else {
                self.draw_row(&rows[index], widths, 0, rows[index].line_count);
            }
            index = end;
        }
    }
    fn rowspan_group_end(rows: &[Row], start: usize) -> usize {
        let mut end = start + 1;
        let mut cursor = start;
        while cursor < end {
            for cell in &rows[cursor].cells {
                end = end.max(cursor + cell.rowspan);
            }
            cursor += 1;
        }
        end.min(rows.len())
    }
    fn rowspan_group_heights(rows: &[Row], start: usize, end: usize) -> Vec<f32> {
        let mut heights = rows[start..end]
            .iter()
            .map(Self::table_row_height)
            .collect::<Vec<_>>();
        for (row_index, row) in rows.iter().enumerate().take(end).skip(start) {
            for cell in &row.cells {
                if cell.rowspan <= 1 {
                    continue;
                }
                let span_end = row_index + cell.rowspan;
                let local_start = row_index - start;
                let local_end = span_end - start;
                let natural = cell.content_height
                    + cell.style.padding[0]
                    + cell.style.padding[2]
                    + 2.0 * cell.style.border_width;
                let requested =
                    resolved_dimension(cell.style.height, cell.style.height_percent, natural)
                        .unwrap_or(natural);
                let required = constrained_dimension(
                    requested,
                    resolved_dimension(
                        cell.style.min_height,
                        cell.style.min_height_percent,
                        natural,
                    ),
                    resolved_dimension(
                        cell.style.max_height,
                        cell.style.max_height_percent,
                        natural,
                    ),
                )
                .max(natural);
                let current: f32 = heights[local_start..local_end].iter().sum::<f32>()
                    - rows[span_end - 1].row_gap;
                if required > current {
                    heights[local_end - 1] += required - current;
                }
            }
        }
        heights
    }
    fn draw_rowspan_group(
        &mut self,
        rows: &[Row],
        widths: &[f32],
        start: usize,
        end: usize,
        heights: &[f32],
    ) {
        let height: f32 = heights.iter().sum();
        self.draw_rowspan_fragment(rows, widths, start, end, heights, 0.0..height);
    }
    fn rowspan_offsets(heights: &[f32]) -> Vec<f32> {
        let mut offsets = Vec::with_capacity(heights.len() + 1);
        offsets.push(0.0);
        for height in heights {
            offsets.push(offsets.last().copied().unwrap_or(0.0) + height);
        }
        offsets
    }
    fn rowspan_fragment_end(
        rows: &[Row],
        start: usize,
        end: usize,
        heights: &[f32],
        fragment_start: f32,
        desired_end: f32,
    ) -> Result<f32> {
        let offsets = Self::rowspan_offsets(heights);
        let mut fragment_end = desired_end;
        loop {
            let mut adjusted = fragment_end;
            for (row_index, row) in rows.iter().enumerate().take(end).skip(start) {
                let local_row = row_index - start;
                for cell in &row.cells {
                    let span_end = local_row + cell.rowspan;
                    let cell_height = offsets[span_end] - offsets[local_row] - row.row_gap;
                    let content_top = offsets[local_row]
                        + cell.style.border_width
                        + cell.style.padding[0]
                        + Self::table_cell_vertical_offset(
                            &cell.style,
                            cell_height,
                            cell.content_height,
                        );
                    if cell.items.is_empty() {
                        let mut line_top = content_top;
                        for line in &cell.lines {
                            let line_bottom = line_top + line.height;
                            if line_top < adjusted - EPS && line_bottom > adjusted + EPS {
                                adjusted = adjusted.min(line_top);
                            }
                            line_top = line_bottom;
                        }
                    } else {
                        let content_bottom = content_top + cell.content_height;
                        if content_top < adjusted - EPS && content_bottom > adjusted + EPS {
                            adjusted = adjusted.min(content_top);
                        }
                    }
                }
            }
            if (adjusted - fragment_end).abs() <= EPS {
                break;
            }
            fragment_end = adjusted;
        }
        if fragment_end <= fragment_start + EPS {
            return Err(Error(
                "table rowspan content line exceeds available page area".into(),
            ));
        }
        Ok(fragment_end)
    }
    fn draw_rowspan_fragment(
        &mut self,
        rows: &[Row],
        widths: &[f32],
        start: usize,
        end: usize,
        heights: &[f32],
        fragment: std::ops::Range<f32>,
    ) {
        let fragment_start = fragment.start;
        let fragment_end = fragment.end;
        let page_y = self.y;
        let offsets = Self::rowspan_offsets(heights);
        for (row_index, row) in rows.iter().enumerate().take(end).skip(start) {
            let local_row = row_index - start;
            for cell in &row.cells {
                let x = row.x
                    + widths[..cell.column].iter().sum::<f32>()
                    + row.column_gap * cell.column as f32;
                let width: f32 = widths[cell.column..cell.column + cell.colspan]
                    .iter()
                    .sum::<f32>()
                    + row.column_gap * cell.colspan.saturating_sub(1) as f32;
                let span_end = local_row + cell.rowspan;
                let cell_start = offsets[local_row];
                let cell_end = offsets[span_end] - row.row_gap;
                let overlap_start = cell_start.max(fragment_start);
                let overlap_end = cell_end.min(fragment_end);
                if overlap_end <= overlap_start + EPS {
                    continue;
                }
                let y = page_y + overlap_start - fragment_start;
                let height = overlap_end - overlap_start;
                self.paint_table_cell(cell, (x, y, width, height), (true, true), row.collapsed);
                let text_x = x + cell.style.border_width + cell.style.padding[3];
                let cell_height = cell_end - cell_start;
                let content_top = cell_start
                    + cell.style.border_width
                    + cell.style.padding[0]
                    + Self::table_cell_vertical_offset(
                        &cell.style,
                        cell_height,
                        cell.content_height,
                    );
                let inner_width = width
                    - cell.style.padding[1]
                    - cell.style.padding[3]
                    - 2.0 * cell.style.border_width;
                let mut line_top = content_top;
                for (line_index, line) in cell.lines.iter().enumerate() {
                    let line_bottom = line_top + line.height;
                    if line_top >= fragment_start - EPS && line_bottom <= fragment_end + EPS {
                        self.draw_line(
                            line,
                            text_x,
                            page_y + line_top - fragment_start,
                            inner_width,
                            &cell.style.text_align,
                            line_index + 1 == cell.lines.len(),
                        );
                    }
                    line_top = line_bottom;
                }
                if !cell.items.is_empty() {
                    let content_bottom = content_top + cell.content_height;
                    if content_top >= fragment_start - EPS && content_bottom <= fragment_end + EPS {
                        self.draw_cell_items(cell, text_x, page_y + content_top - fragment_start);
                    }
                }
            }
        }
        self.y += fragment_end - fragment_start;
    }
    fn table_cell_vertical_offset(style: &Style, box_height: f32, content_height: f32) -> f32 {
        let available = (box_height
            - style.padding[0]
            - style.padding[2]
            - 2.0 * style.border_width
            - content_height)
            .max(0.0);
        match style.vertical_align.as_str() {
            "middle" => available / 2.0,
            "bottom" => available,
            _ => 0.0,
        }
    }
    fn ensure_header(
        &mut self,
        heads: &[Row],
        widths: &[f32],
        height: f32,
        footer_height: f32,
        row: &Row,
    ) -> Result<()> {
        let first_line = row.step + row.pad + row.row_gap;
        if height + first_line + footer_height > self.full_height() + EPS {
            return Err(Error(
                "table header and footer leave no room for a data line".into(),
            ));
        }
        let complete_row = Self::table_row_height(row);
        let data_height = if height + complete_row + footer_height <= self.full_height() + EPS {
            complete_row
        } else {
            first_line
        };
        if self.y + height + data_height + footer_height > self.limit() + EPS
            && self.page_has_content()
        {
            self.new_page()?;
        }
        self.draw_table_section(heads, widths);
        Ok(())
    }
    fn draw_table_footer(&mut self, foots: &[Row], widths: &[f32]) {
        self.draw_table_section(foots, widths);
    }
    // A row fragment consumes at least one line. Every continuation starts with the header.
    fn draw_row(&mut self, row: &Row, widths: &[f32], start: usize, count: usize) {
        let height = count as f32 * row.step + row.pad;
        let y = self.y;
        for cell in &row.cells {
            let x = row.x
                + widths[..cell.column].iter().sum::<f32>()
                + row.column_gap * cell.column as f32;
            let w: f32 = widths[cell.column..cell.column + cell.colspan]
                .iter()
                .sum::<f32>()
                + row.column_gap * cell.colspan.saturating_sub(1) as f32;
            self.paint_table_cell(cell, (x, y, w, height), (true, true), row.collapsed);
            let text_x = x + cell.style.border_width + cell.style.padding[3];
            let visible_lines = cell.lines.len().saturating_sub(start).min(count);
            let content_height = if cell.items.is_empty() {
                visible_lines as f32 * row.step
            } else {
                cell.content_height
            };
            let vertical_offset = if start == 0 && count >= cell.lines.len() {
                Self::table_cell_vertical_offset(&cell.style, height, content_height)
            } else {
                0.0
            };
            let mut line_y = y + cell.style.border_width + cell.style.padding[0] + vertical_offset;
            for (local_index, line) in cell.lines.iter().skip(start).take(count).enumerate() {
                self.draw_line(
                    line,
                    text_x,
                    line_y,
                    w - cell.style.padding[1]
                        - cell.style.padding[3]
                        - 2.0 * cell.style.border_width,
                    &cell.style.text_align,
                    start + local_index + 1 == cell.lines.len(),
                );
                line_y += row.step;
            }
            if !cell.items.is_empty() && start == 0 {
                self.draw_cell_items(cell, text_x, line_y);
            }
        }
        self.y += height;
        if start + count >= row.line_count {
            self.y += row.row_gap;
        }
    }
}
