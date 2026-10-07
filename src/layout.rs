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

fn decoded_reference(raw: &str) -> Option<char> {
    match raw {
        "nbsp" => Some('\u{00a0}'),
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => raw
            .strip_prefix("#x")
            .or_else(|| raw.strip_prefix("#X"))
            .and_then(|value| u32::from_str_radix(value, 16).ok())
            .or_else(|| raw.strip_prefix('#').and_then(|value| value.parse().ok()))
            .and_then(char::from_u32),
    }
}
fn source_character_offset(html: &str, wanted: char) -> Option<usize> {
    let literal = html.find(wanted);
    let mut reference = None;
    for (start, _) in html.match_indices('&') {
        let tail = &html[start + 1..];
        let Some(end) = tail.find(';') else { continue };
        if end <= 32 && decoded_reference(&tail[..end]) == Some(wanted) {
            reference = Some(start);
            break;
        }
    }
    match (literal, reference) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
fn add_html_location(error: Error, html: &str, source_name: Option<&str>) -> Error {
    if error.0.contains("; location ") {
        return error;
    }
    let offset = if let Some(hex) = error
        .0
        .strip_prefix("no registered font for U+")
        .and_then(|details| details.split_once(' ').map(|(hex, _)| hex))
    {
        let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) else {
            return error;
        };
        source_character_offset(html, ch)
    } else if error.0.contains("HTML body not found") {
        html.find('<')
    } else if let Some(target) = error
        .0
        .strip_prefix("strict mode: pdf-link: unsupported PDF link target: ")
    {
        let encoded = target
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        html.find(target).or_else(|| html.find(&encoded))
    } else {
        let opening_tag = error
            .0
            .split_once('<')
            .and_then(|(_, rest)| rest.split_once('>'))
            .map(|(tag, _)| format!("<{tag}"));
        opening_tag.and_then(|tag| {
            let lower = html.to_ascii_lowercase();
            lower.find(&tag.to_ascii_lowercase())
        })
    };
    let Some((line, column)) = offset.and_then(|offset| source_location_at(html, offset)) else {
        return error;
    };
    Error(format!(
        "{}; location {} {line} {column}",
        error.0,
        source_name.unwrap_or("<html>")
    ))
}

fn add_html_conversion_error_location(
    error: Error,
    html: &str,
    css_text: &str,
    source_name: Option<&str>,
) -> Error {
    let error = add_html_location(error, html, source_name);
    if error.0.contains("; location ") {
        return error;
    }
    let Some(details) = error.0.strip_prefix("strict mode: ") else {
        return error;
    };
    let Some((code, message)) = details.split_once(": ") else {
        return error;
    };
    if code != "css-property" {
        return error;
    }
    let mut diagnostic = Diagnostic::new("css-property", message);
    if let Some(declaration) = message.strip_prefix("unsupported CSS declaration: ") {
        if let Some((property, value)) = declaration.split_once(": ") {
            diagnostic.property = Some(property.to_owned());
            diagnostic.value = Some(value.to_owned());
        }
    } else if let Some((property, value)) = message.split_once(": ") {
        if matches!(property, "position" | "break-inside") {
            diagnostic.property = Some(property.to_owned());
            diagnostic.value = Some(value.split_whitespace().next().unwrap_or(value).to_owned());
        } else if property == "transform" {
            diagnostic.property = Some(property.to_owned());
        }
    }
    diagnostic_location(&mut diagnostic, html, css_text, source_name);
    let (Some(source), Some(line), Some(column)) =
        (diagnostic.source, diagnostic.line, diagnostic.column)
    else {
        return error;
    };
    Error(format!("{}; location {source} {line} {column}", error.0))
}

fn add_missing_glyph_source_location(error: Error, source: &str, label: &str) -> Error {
    if error.0.contains("; location ") {
        return error;
    }
    let Some(hex) = error
        .0
        .strip_prefix("no registered font for U+")
        .and_then(|details| details.split_once(' ').map(|(hex, _)| hex))
    else {
        return error;
    };
    let Some(character) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) else {
        return error;
    };
    let Some(offset) = source_character_offset(source, character) else {
        return error;
    };
    let Some((line, column)) = source_location_at(source, offset) else {
        return error;
    };
    Error(format!("{}; location {label} {line} {column}", error.0))
}

#[derive(Clone)]
struct SourceContext {
    html: Arc<str>,
    source_name: Option<Arc<str>>,
}

impl SourceContext {
    fn new(html: &str, source_name: Option<String>) -> Self {
        Self {
            html: Arc::from(html),
            source_name: source_name.map(Arc::from),
        }
    }
}

fn add_node_location(error: Error, offset: Option<usize>, source: &SourceContext) -> Error {
    // Missing-glyph locations point to the actual character (including entity
    // references), which is more precise than the containing element span.
    if error.0.contains("; location ") || error.0.starts_with("no registered font for U+") {
        return error;
    }
    let Some((line, column)) = offset.and_then(|offset| source_location_at(&source.html, offset))
    else {
        return error;
    };
    Error(format!(
        "{}; location {} {line} {column}",
        error.0,
        source.source_name.as_deref().unwrap_or("<html>")
    ))
}

fn add_text_token_location(
    error: Error,
    token: &str,
    html: &str,
    css_text: &str,
    source_name: Option<&str>,
) -> Error {
    if error.0.contains("; location ") || token.is_empty() {
        return error;
    }
    let location = find_source_location(css_text, token)
        .map(|position| ("<css>", position))
        .or_else(|| {
            find_source_location(html, token)
                .map(|position| (source_name.unwrap_or("<html>"), position))
        });
    let Some((source, (line, column))) = location else {
        return error;
    };
    Error(format!("{}; location {source} {line} {column}", error.0))
}

fn add_css_error_location(
    error: Error,
    html: &str,
    css_text: &str,
    all_css: &str,
    source_name: Option<&str>,
) -> Error {
    if error.0.contains("; location ") {
        return error;
    }
    let needle = if error.0.contains("unclosed CSS comment") {
        "/*".to_owned()
    } else if let Some((_, details)) = error.0.rsplit_once("unsupported at-rule: ") {
        details
            .split([' ', '{'])
            .next()
            .unwrap_or_default()
            .to_owned()
    } else if let Some((_, details)) = error.0.rsplit_once("unsupported selector: ") {
        details
            .split([' ', '{'])
            .next()
            .unwrap_or_default()
            .to_owned()
    } else {
        error
            .0
            .split(": ")
            .last()
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    if let Some(position) = find_source_location(css_text, &needle) {
        return Error(format!(
            "{}; location <css> {} {}",
            error.0, position.0, position.1
        ));
    }
    if let Some(position) = find_source_location(html, &needle) {
        return Error(format!(
            "{}; location {} {} {}",
            error.0,
            source_name.unwrap_or("<html>"),
            position.0,
            position.1
        ));
    }
    if let Some(position) = find_source_location(all_css, &needle) {
        return Error(format!(
            "{}; location <css> {} {}",
            error.0, position.0, position.1
        ));
    }
    error
}

fn find_source_location(source: &str, needle: &str) -> Option<(usize, usize)> {
    let offset = source.find(needle)?;
    source_location_at(source, offset)
}

fn source_location_at(source: &str, offset: usize) -> Option<(usize, usize)> {
    if !source.is_char_boundary(offset) {
        return None;
    }
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |last_line| last_line.chars().count() + 1);
    Some((line, column))
}

fn find_source_location_ascii_case_insensitive(
    source: &str,
    needle: &str,
) -> Option<(usize, usize)> {
    let offset = source
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())?;
    source_location_at(source, offset)
}

fn find_embedded_css_location(html: &str, needle: &str) -> Option<(usize, usize)> {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("<style") {
        let start = cursor + relative;
        let after_name = start + "<style".len();
        if lower
            .as_bytes()
            .get(after_name)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>')
        {
            cursor = after_name;
            continue;
        }
        let body_start = lower[after_name..].find('>')? + after_name + 1;
        let body_end = lower[body_start..].find("</style")? + body_start;
        if let Some(relative) = html[body_start..body_end].find(needle) {
            return source_location_at(html, body_start + relative);
        }
        cursor = body_end + "</style".len();
    }
    None
}

fn find_declaration_offset(source: &str, property: &str, value: &str) -> Option<usize> {
    let lower = source.to_ascii_lowercase();
    let property_lower = property.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find(&property_lower) {
        let start = cursor + relative;
        let before = source[..start].chars().next_back();
        let mut after_property = start + property.len();
        while source
            .as_bytes()
            .get(after_property)
            .is_some_and(u8::is_ascii_whitespace)
        {
            after_property += 1;
        }
        let boundary = before.is_none_or(|ch| ch.is_ascii_whitespace() || matches!(ch, '{' | ';'));
        if boundary && source.as_bytes().get(after_property) == Some(&b':') {
            let last_comment = source[..start].rfind("/*");
            let last_comment_end = source[..start].rfind("*/");
            let outside_comment =
                last_comment.is_none_or(|open| last_comment_end.is_some_and(|close| close > open));
            if outside_comment {
                let value_start = after_property + 1;
                let value_end = source[value_start..]
                    .find([';', '}'])
                    .map_or(source.len(), |offset| value_start + offset);
                let actual = source[value_start..value_end]
                    .trim()
                    .trim_end_matches("!important")
                    .trim();
                if actual.eq_ignore_ascii_case(value.trim()) {
                    return Some(start);
                }
            }
        }
        cursor = start + property.len();
    }
    None
}

fn find_inline_style_declaration_offset(html: &str, property: &str, value: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let relative = lower[cursor..].find('<')?;
        let open = cursor + relative;
        if lower[open..].starts_with("<!--") {
            cursor = lower[open + 4..]
                .find("-->")
                .map_or(bytes.len(), |end| open + 4 + end + 3);
            continue;
        }
        let mut index = open + 1;
        if bytes
            .get(index)
            .is_some_and(|byte| matches!(byte, b'/' | b'!' | b'?'))
        {
            cursor = lower[index..]
                .find('>')
                .map_or(bytes.len(), |end| index + end + 1);
            continue;
        }
        let name_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_alphanumeric) {
            index += 1;
        }
        if name_start == index {
            cursor = open + 1;
            continue;
        }
        let tag = &lower[name_start..index];
        let mut tag_end = index;
        let mut quote = None;
        while tag_end < bytes.len() {
            let byte = bytes[tag_end];
            if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() && matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if quote.is_none() && byte == b'>' {
                break;
            }
            tag_end += 1;
        }
        if tag_end >= bytes.len() {
            return None;
        }

        while index < tag_end {
            while index < tag_end
                && (bytes[index].is_ascii_whitespace() || matches!(bytes[index], b'/'))
            {
                index += 1;
            }
            let attribute_start = index;
            while index < tag_end
                && !bytes[index].is_ascii_whitespace()
                && !matches!(bytes[index], b'=' | b'/' | b'>')
            {
                index += 1;
            }
            if attribute_start == index {
                index += 1;
                continue;
            }
            let is_style = lower[attribute_start..index] == *"style";
            while index < tag_end && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if bytes.get(index) != Some(&b'=') {
                continue;
            }
            index += 1;
            while index < tag_end && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index >= tag_end {
                break;
            }
            let (value_start, value_end) = if matches!(bytes[index], b'\'' | b'"') {
                let delimiter = bytes[index];
                index += 1;
                let start = index;
                while index < tag_end && bytes[index] != delimiter {
                    index += 1;
                }
                let end = index;
                index = (index + 1).min(tag_end);
                (start, end)
            } else {
                let start = index;
                while index < tag_end && !bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                (start, index)
            };
            if is_style {
                let inline = &html[value_start..value_end];
                if let Some(offset) = find_declaration_offset(inline, property, value) {
                    return Some(value_start + offset);
                }
            }
        }
        cursor = tag_end + 1;
        if matches!(
            tag,
            "script"
                | "style"
                | "textarea"
                | "title"
                | "xmp"
                | "iframe"
                | "noembed"
                | "noframes"
                | "plaintext"
        ) {
            let closing = format!("</{tag}");
            cursor = lower[cursor..]
                .find(&closing)
                .map_or(bytes.len(), |offset| cursor + offset);
        }
    }
    None
}

fn diagnostic_location(
    diagnostic: &mut Diagnostic,
    html: &str,
    css_text: &str,
    source_name: Option<&str>,
) {
    if diagnostic.line.is_some() && diagnostic.column.is_some() {
        return;
    }
    let html_source = source_name.unwrap_or("<html>");
    let mut needles = Vec::<String>::new();
    if let Some(property) = diagnostic.property.as_deref() {
        needles.push(format!("{property}:"));
        needles.push(property.to_owned());
    }
    if let Some(selector) = diagnostic.selector.as_deref() {
        needles.push(selector.to_owned());
    }
    match diagnostic.code {
        "html-tag" => {
            if let Some(tag) = diagnostic
                .message
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>'))
                .map(|(tag, _)| format!("<{tag}"))
            {
                if let Some(position) = find_source_location_ascii_case_insensitive(html, &tag) {
                    diagnostic.source = Some(html_source.to_owned());
                    diagnostic.line = Some(position.0);
                    diagnostic.column = Some(position.1);
                    return;
                }
            }
            if diagnostic.message.contains("<script>") {
                needles.push("<script".into());
            }
        }
        "pdf-link" => {
            if let Some((_, target)) = diagnostic.message.split_once(": ") {
                needles.push(target.to_owned());
                needles.push(
                    target
                        .replace('&', "&amp;")
                        .replace('"', "&quot;")
                        .replace('\'', "&#39;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                );
            }
        }
        "css-property" => {
            if let (Some(property), Some(value)) =
                (diagnostic.property.as_deref(), diagnostic.value.as_deref())
            {
                if let Some((source, position)) = find_declaration_offset(css_text, property, value)
                    .and_then(|offset| {
                        source_location_at(css_text, offset).map(|pos| ("<css>", pos))
                    })
                    .or_else(|| {
                        find_inline_style_declaration_offset(html, property, value).and_then(
                            |offset| source_location_at(html, offset).map(|pos| (html_source, pos)),
                        )
                    })
                {
                    diagnostic.source = Some(source.to_owned());
                    diagnostic.line = Some(position.0);
                    diagnostic.column = Some(position.1);
                    return;
                }
            }
            if diagnostic.property.is_none() {
                if let Some((_, token)) = diagnostic.message.split_once(": ") {
                    let token = token.trim();
                    if !token.is_empty() {
                        needles.push(token.to_owned());
                        if let Some((first, _)) = token.split_once([' ', ':', ';']) {
                            if !first.is_empty() {
                                needles.push(first.to_owned());
                            }
                        }
                    }
                }
            }
            if diagnostic.message.contains("image borders and backgrounds") {
                needles.extend(["background".into(), "border".into()]);
            }
            if diagnostic.message.contains("page break is unsupported") {
                needles.extend([
                    "break-before".into(),
                    "break-after".into(),
                    "page-break-before".into(),
                    "page-break-after".into(),
                ]);
            }
            if diagnostic.message.contains("transform") {
                needles.push("transform".into());
            }
            if diagnostic.message.contains("position:") {
                needles.push("position".into());
            }
            if diagnostic.message.contains("break-inside") {
                needles.push("break-inside".into());
            }
        }
        "css-value" | "css-syntax" | "at-rule" | "selector" | "font-face" => {
            if diagnostic.code == "font-face" {
                needles.push("@font-face".into());
            }
            if let Some((_, token)) = diagnostic.message.split_once(": ") {
                let token = token.trim();
                if !token.is_empty() {
                    needles.push(token.to_owned());
                    if let Some((first, _)) = token.split_once([' ', ':', ';']) {
                        if !first.is_empty() {
                            needles.push(first.to_owned());
                        }
                    }
                }
            }
        }
        _ => {}
    }
    needles.dedup();
    let location = needles.iter().find_map(|needle| {
        find_source_location(css_text, needle)
            .map(|position| ("<css>", position))
            .or_else(|| {
                find_embedded_css_location(html, needle).map(|position| (html_source, position))
            })
            .or_else(|| find_source_location(html, needle).map(|position| (html_source, position)))
    });
    if let Some((source, (line, column))) = location {
        diagnostic.source = Some(source.to_owned());
        diagnostic.line = Some(line);
        diagnostic.column = Some(column);
    }
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
fn data_resource(src: &str, remaining: usize) -> Result<Vec<u8>> {
    let (header, payload) = src
        .strip_prefix("data:")
        .and_then(|value| value.split_once(','))
        .ok_or_else(|| Error("invalid resource data URI".into()))?;
    if !header
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return Err(Error("resource data URI requires base64 encoding".into()));
    }
    if payload.len() > (remaining.saturating_mul(4) / 3).saturating_add(8) {
        return Err(Error("render limit exceeded: max_resource_bytes".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| Error(format!("invalid resource data URI base64: {e}")))?;
    if bytes.len() > remaining {
        return Err(Error("render limit exceeded: max_resource_bytes".into()));
    }
    Ok(bytes)
}
/// Prepared vector PDF objects retained after layout.
#[derive(Clone)]
pub struct SvgData {
    pub chunk: pdf_writer::Chunk,
    pub root: pdf_writer::Ref,
}
impl std::fmt::Debug for SvgData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SvgData")
            .field("bytes", &self.chunk.len())
            .field("root", &self.root.get())
            .finish()
    }
}

/// A fixed RGB raster loaded at layout time, optionally with vector SVG source.
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
    pub svg: Option<SvgData>,
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
    for font_source in &font_sources {
        options.fontdb_mut().load_font_data(font_source.clone());
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
    let mut vector_options = svg2pdf::usvg::Options::default();
    for font_source in &font_sources {
        vector_options
            .fontdb_mut()
            .load_font_data(font_source.clone());
    }
    let vector_tree = svg2pdf::usvg::Tree::from_data(bytes, &vector_options)
        .map_err(|e| Error(format!("SVG vector parse: {e}")))?;
    let conversion = svg2pdf::ConversionOptions {
        raster_scale: dpi / 96.0,
        embed_text: false,
        ..svg2pdf::ConversionOptions::default()
    };
    let (chunk, root) = svg2pdf::to_chunk(&vector_tree, conversion)
        .map_err(|e| Error(format!("SVG vector export: {e}")))?;
    Ok(ImageData {
        rgb,
        alpha: alpha.iter().any(|a| *a != 255).then_some(alpha),
        jpeg: None,
        width,
        height,
        display_width: intrinsic.width() * 0.75,
        display_height: intrinsic.height() * 0.75,
        digest,
        svg: Some(SvgData { chunk, root }),
    })
}
impl ImageData {
    fn retained_bytes(&self) -> usize {
        self.rgb.len()
            + self.alpha.as_ref().map_or(0, Vec::len)
            + self.jpeg.as_ref().map_or(0, Vec::len)
            + self.svg.as_ref().map_or(0, |svg| svg.chunk.len())
    }
}
fn downsample_image(data: &ImageData, max_dpi: f32, width: f32, height: f32) -> Result<ImageData> {
    let target_width = ((width / 72.0) * max_dpi).ceil().max(1.0) as u32;
    let target_height = ((height / 72.0) * max_dpi).ceil().max(1.0) as u32;
    if target_width >= data.width && target_height >= data.height {
        return Err(Error("image does not require downsampling".into()));
    }
    let image = if let Some(jpeg) = &data.jpeg {
        image::load_from_memory(jpeg).map_err(|e| Error(format!("image downsample decode: {e}")))?
    } else {
        let mut rgba = Vec::with_capacity(data.width as usize * data.height as usize * 4);
        for (index, rgb) in data.rgb.as_chunks::<3>().0.iter().enumerate() {
            rgba.extend_from_slice(rgb);
            rgba.push(data.alpha.as_ref().map_or(255, |alpha| alpha[index]));
        }
        image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(data.width, data.height, rgba)
                .ok_or_else(|| Error("invalid raster buffer for downsampling".into()))?,
        )
    };
    let resized = image.resize_exact(
        target_width.min(data.width),
        target_height.min(data.height),
        image::imageops::FilterType::Lanczos3,
    );
    let rgba = resized.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    let mut alpha = Vec::with_capacity(width as usize * height as usize);
    for pixel in rgba.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel[3]);
    }
    let alpha = alpha.iter().any(|value| *value != 255).then_some(alpha);
    let mut hash = Sha256::new();
    hash.update(data.digest);
    hash.update(b"downsample");
    hash.update(width.to_be_bytes());
    hash.update(height.to_be_bytes());
    Ok(ImageData {
        rgb,
        alpha,
        jpeg: None,
        width,
        height,
        display_width: data.display_width,
        display_height: data.display_height,
        digest: hash.finalize().into(),
        svg: None,
    })
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
    RoundedRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Color,
        radius: [f32; 4],
        alpha: f32,
    },
    Border {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        widths: [f32; 4],
        colors: [Color; 4],
        alphas: [f32; 4],
        styles: [String; 4],
        radius: [f32; 4],
    },
    Image {
        data: Arc<ImageData>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// A CSS affine transform in top-left page coordinates.
    BeginTransform([f32; 6]),
    EndTransform,
    BeginOpacity(f32),
    EndOpacity,
    BeginClip {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: [f32; 4],
    },
    EndClip,
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
        | Item::RoundedRect { x, y, .. }
        | Item::Border { x, y, .. }
        | Item::Image { x, y, .. }
        | Item::BeginClip { x, y, .. }
        | Item::Link { x, y, .. }
        | Item::Destination { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        Item::BeginTransform(matrix) => {
            // The enclosed primitives are translated too. Conjugating keeps
            // the transform origin attached to those primitives instead of
            // applying the translation twice.
            matrix[4] += dx - matrix[0] * dx - matrix[2] * dy;
            matrix[5] += dy - matrix[1] * dx - matrix[3] * dy;
        }
        Item::BeginActualText(_)
        | Item::EndActualText
        | Item::EndTransform
        | Item::BeginOpacity(_)
        | Item::EndOpacity
        | Item::EndClip => {}
    }
}

fn item_bounds(item: &Item) -> Option<(f32, f32, f32, f32)> {
    match item {
        Item::Text {
            x,
            y,
            size,
            natural_advance,
            ..
        } => Some((*x, *y - *size, natural_advance.max(0.0), *size * 1.2)),
        Item::Rect { x, y, w, h, .. }
        | Item::RoundedRect { x, y, w, h, .. }
        | Item::Border { x, y, w, h, .. }
        | Item::Image { x, y, w, h, .. }
        | Item::BeginClip { x, y, w, h, .. }
        | Item::Link { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
        Item::Destination { x, y, .. } => Some((*x, *y, 0.0, 0.0)),
        _ => None,
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
    pub(crate) outlines: Vec<OutlineEntry>,
}
#[derive(Clone, Debug)]
pub(crate) struct OutlineEntry {
    pub title: String,
    pub destination: String,
    pub level: u8,
}
impl PreparedDocument {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    pub fn to_pdf(&self) -> Result<Vec<u8>> {
        self.to_pdf_with_cancel(None)
    }
    pub fn to_pdf_with_cancel(&self, token: Option<&AtomicBool>) -> Result<Vec<u8>> {
        self.to_pdf_with_options(&crate::pdf::PdfOptions::default(), token)
    }
    pub fn to_pdf_with_options(
        &self,
        options: &crate::pdf::PdfOptions,
        token: Option<&AtomicBool>,
    ) -> Result<Vec<u8>> {
        let start = Instant::now();
        let bytes = crate::pdf::export_with_options(self, options, token)?;
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
    /// Optional source label reported by structured diagnostics.
    pub source_name: Option<String>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub svg_dpi: f32,
    /// Optional upper bound for raster pixels per rendered inch.
    pub max_image_dpi: Option<f32>,
    pub presentational_hints: bool,
    /// Use font bounding boxes for line-box sizing in the WeasyPrint compatibility API.
    pub use_font_bbox_for_line_height: bool,
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
            // Retained for source compatibility. Shaping is unconditional in 0.2.0.
            experimental_shaping: true,
            source_name: None,
            synthetic_bold: false,
            synthetic_italic: false,
            svg_dpi: 144.0,
            max_image_dpi: None,
            presentational_hints: false,
            use_font_bbox_for_line_height: false,
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
        // CSS supplied alongside the HTML behaves like a user stylesheet in
        // the WeasyPrint API, so normal author declarations in the document win.
        let all_css = format!("{}\n{}", css_text, html::embedded_css(html));
        let sheet = css::parse(&all_css, self.strict).map_err(|error| {
            add_css_error_location(error, html, css_text, &all_css, self.source_name.as_deref())
        })?;
        let css_cascade_ms = css_start.elapsed().as_secs_f64() * 1000.0;
        if sheet.rules.len() > self.limits.max_css_rules {
            return Err(Error("render limit exceeded: max_css_rules".into()));
        }
        if sheet.resolved_page_styles().iter().any(|page| {
            page.width <= page.margin[1] + page.margin[3]
                || page.height <= page.margin[0] + page.margin[2]
                || page.margin.iter().any(|value| *value < 0.0)
        }) {
            return Err(add_text_token_location(
                Error("@page margins leave no positive content rectangle".into()),
                "@page",
                html,
                css_text,
                self.source_name.as_deref(),
            ));
        }
        let fonts = self.fonts.snapshot();
        fonts.set_synthetic(self.synthetic_bold, self.synthetic_italic);
        let mut font_resource_bytes = 0usize;
        // Local @font-face files are snapshotted before layout, never at PDF export.
        for face in &sheet.faces {
            let src = face.src.trim();
            let Some(path) = src.strip_prefix("url(").and_then(|s| s.strip_suffix(')')) else {
                return Err(add_text_token_location(
                    Error(format!("unsupported @font-face src: {src}")),
                    src,
                    html,
                    css_text,
                    self.source_name.as_deref(),
                ));
            };
            let path = path.trim().trim_matches(['\'', '"']);
            if path.starts_with("data:") {
                let bytes = data_resource(
                    path,
                    self.limits
                        .max_resource_bytes
                        .saturating_sub(font_resource_bytes),
                )
                .map_err(|error| {
                    add_text_token_location(
                        error,
                        path,
                        html,
                        css_text,
                        self.source_name.as_deref(),
                    )
                })?;
                font_resource_bytes = font_resource_bytes.saturating_add(bytes.len());
                fonts
                    .register_bytes(bytes, &face.family, face.weight, &face.style)
                    .map_err(|error| {
                        add_text_token_location(
                            error,
                            path,
                            html,
                            css_text,
                            self.source_name.as_deref(),
                        )
                    })?;
                continue;
            }
            if path.contains("://") {
                return Err(add_text_token_location(
                    Error("@font-face supports local file URLs only".into()),
                    path,
                    html,
                    css_text,
                    self.source_name.as_deref(),
                ));
            }
            let resolved = local_resource_path(&self.base_dir, path, self.limits.restrict_base_dir)
                .map_err(|error| {
                    add_text_token_location(
                        error,
                        path,
                        html,
                        css_text,
                        self.source_name.as_deref(),
                    )
                })?;
            let size = std::fs::metadata(&resolved)
                .map_err(|e| {
                    add_text_token_location(
                        Error(format!("font {}: {e}", resolved.display())),
                        path,
                        html,
                        css_text,
                        self.source_name.as_deref(),
                    )
                })?
                .len() as usize;
            font_resource_bytes = font_resource_bytes.saturating_add(size);
            if font_resource_bytes > self.limits.max_resource_bytes {
                return Err(add_text_token_location(
                    Error("render limit exceeded: max_resource_bytes".into()),
                    path,
                    html,
                    css_text,
                    self.source_name.as_deref(),
                ));
            }
            cancelled(token.as_deref())?;
            fonts
                .register_file(
                    &resolved.to_string_lossy(),
                    &face.family,
                    face.weight,
                    &face.style,
                )
                .map_err(|error| {
                    add_text_token_location(
                        error,
                        path,
                        html,
                        css_text,
                        self.source_name.as_deref(),
                    )
                })?;
        }
        let parse_start = Instant::now();
        let (mut root, warnings, metadata) = html::parse_with_metadata_and_hints(
            html,
            &sheet,
            self.strict,
            self.presentational_hints,
        )
        .map_err(|error| {
            add_html_conversion_error_location(error, html, css_text, self.source_name.as_deref())
        })?;
        if self.use_font_bbox_for_line_height {
            fn enable_font_bbox(node: &mut Node) {
                node.style.use_font_bbox_for_line_height = true;
                for child in &mut node.children {
                    enable_font_bbox(child);
                }
            }
            enable_font_bbox(&mut root);
        }
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
        let layout_start = Instant::now();
        let mut flow = Flow::new(
            sheet.clone(),
            fonts,
            self.base_dir.clone(),
            self.images.clone(),
            warnings,
            self.limits.clone(),
            token,
            SourceContext::new(html, self.source_name.clone()),
        );
        flow.experimental_shaping = true;
        flow.svg_dpi = self.svg_dpi;
        flow.max_image_dpi = self.max_image_dpi;
        flow.resource_bytes = font_resource_bytes;
        flow.node(&root)
            .map_err(|error| add_html_location(error, html, self.source_name.as_deref()))?;
        flow.render_fixed_nodes()
            .map_err(|error| add_html_location(error, html, self.source_name.as_deref()))?;
        flow.compose_positioned_layers();
        flow.render_margin_boxes().map_err(|error| {
            let css_error = add_missing_glyph_source_location(error, css_text, "<css>");
            add_html_location(css_error, html, self.source_name.as_deref())
        })?;
        let mut diagnostics: Vec<Diagnostic> = Vec::new();
        let mut seen: HashMap<(&'static str, String), usize> = HashMap::new();
        for mut warning in flow.warnings {
            diagnostic_location(&mut warning, html, css_text, self.source_name.as_deref());
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
            outlines: flow.outlines,
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
    line_height: f32,
    use_font_bbox_for_line_height: bool,
    color: Color,
    alpha: f32,
    shift: f32,
    decoration: TextDecoration,
    href: Option<Arc<str>>,
    preserve_space: bool,
    visible: bool,
    soft_hyphen: bool,
    soft_hyphen_advance: f32,
    inline_box: Option<Arc<InlineBox>>,
    inline_strut: Option<Arc<InlineStrut>>,
    destination: Option<String>,
    paint_late: bool,
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
    destination: Option<String>,
    inline_struts: Vec<Style>,
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
#[derive(Clone, Copy)]
struct InlineStrut {
    ascent: f32,
    descent: f32,
    height: f32,
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
    let ascent = glyphs
        .iter()
        .map(|g| {
            g.inline_box.as_ref().map_or_else(
                || g.size * 0.9 + g.shift,
                |inline| {
                    (inline.style.margin[0] + inline.height + g.shift).max(
                        g.inline_strut
                            .as_ref()
                            .map_or(0.0, |strut| strut.ascent + g.shift),
                    )
                },
            )
        })
        .fold(default_height * 0.75, f32::max);
    let descent = glyphs
        .iter()
        .map(|g| {
            g.inline_box.as_ref().map_or_else(
                || g.size * 0.3 - g.shift,
                |inline| {
                    (inline.style.margin[2] - g.shift).max(
                        g.inline_strut
                            .as_ref()
                            .map_or(0.0, |strut| strut.descent - g.shift),
                    )
                },
            )
        })
        .fold(0.0, f32::max);
    let metric_height = ascent + descent;
    let baseline_ascent = glyphs
        .iter()
        .map(|g| {
            g.inline_box
                .as_ref()
                .map_or_else(
                    || g.font.ascent as f32 / g.font.units_per_em as f32 * g.size + g.shift,
                    |inline| inline.style.margin[0] + inline.height + g.shift,
                )
                .max(
                    g.inline_strut
                        .as_ref()
                        .map_or(0.0, |strut| strut.ascent + g.shift),
                )
        })
        .fold(default_height * 0.75, f32::max);
    let baseline_descent = glyphs
        .iter()
        .map(|g| {
            g.inline_box
                .as_ref()
                .map_or_else(
                    || -(g.font.descent as f32 / g.font.units_per_em as f32 * g.size) - g.shift,
                    |inline| inline.style.margin[2] - g.shift,
                )
                .max(
                    g.inline_strut
                        .as_ref()
                        .map_or(0.0, |strut| strut.descent - g.shift),
                )
        })
        .fold(0.0, f32::max);
    let height = glyphs
        .iter()
        .map(|g| g.line_height)
        .fold(default_height.max(metric_height), f32::max)
        .max(
            glyphs
                .iter()
                .filter_map(|g| g.inline_strut.as_ref().map(|strut| strut.height))
                .fold(0.0, f32::max),
        );
    // Keep line-box sizing compatible with the existing layout rules, but
    // place glyphs using the face's actual vertical metrics. The old 0.9/0.3
    // estimate shifted text upward relative to WeasyPrint even when both used
    // the same font and line-height.
    let baseline = baseline_ascent + ((height - baseline_ascent - baseline_descent) / 2.0);
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
        line_height: style.line_height,
        use_font_bbox_for_line_height: style.use_font_bbox_for_line_height,
        color: style.color,
        alpha: style.color_alpha * style.opacity,
        shift: 0.0,
        decoration: TextDecoration::None,
        href,
        preserve_space,
        visible: style.visibility == "visible",
        soft_hyphen: true,
        soft_hyphen_advance: natural_advance + style.letter_spacing,
        inline_box: None,
        inline_strut: None,
        destination: None,
        paint_late: style.position == "relative",
    })
}
fn inline_runs(
    node: &Node,
    inherited_href: Option<Arc<str>>,
    inherited_destination: Option<String>,
    out: &mut Vec<InlineRun>,
    root: bool,
    inline_struts: &mut Vec<Style>,
) {
    let adds_inline_strut = !root && node.tag != "#text" && node.style.display == "inline";
    if adds_inline_strut {
        inline_struts.push(node.style.clone());
    }
    let mut run_style = node.style.clone();
    if inline_struts
        .iter()
        .any(|ancestor| ancestor.position == "relative")
    {
        run_style.position = "relative".into();
    }
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
    let destination = node
        .attr("id")
        .or_else(|| (node.tag == "a").then(|| node.attr("name")).flatten())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or(inherited_destination);
    if !root && node.tag != "#text" && node.style.display == "inline-block" {
        out.push(InlineRun {
            text: String::new(),
            style: run_style.clone(),
            href,
            inline_box: Some(node.clone()),
            destination,
            inline_struts: inline_struts.clone(),
        });
        if adds_inline_strut {
            inline_struts.pop();
        }
        return;
    }
    if node.tag == "br" {
        out.push(InlineRun {
            text: "\u{000b}".into(),
            style: run_style.clone(),
            href,
            inline_box: None,
            destination,
            inline_struts: inline_struts.clone(),
        });
        if adds_inline_strut {
            inline_struts.pop();
        }
        return;
    }
    if !node.text.is_empty() {
        out.push(InlineRun {
            text: node.text.clone(),
            style: run_style.clone(),
            href: href.clone(),
            inline_box: None,
            destination: destination.clone(),
            inline_struts: inline_struts.clone(),
        });
    } else if destination.is_some() && node.children.is_empty() {
        out.push(InlineRun {
            text: String::new(),
            style: run_style,
            href: href.clone(),
            inline_box: None,
            destination: destination.clone(),
            inline_struts: inline_struts.clone(),
        });
    }
    for child in &node.children {
        inline_runs(
            child,
            href.clone(),
            destination.clone(),
            out,
            false,
            inline_struts,
        );
    }
    if adds_inline_strut {
        inline_struts.pop();
    }
}

fn inline_strut(style: &Style, fonts: &FontRegistry, use_font_bbox: bool) -> Result<InlineStrut> {
    let font = fonts.resolve_with_stretch(
        &style.family,
        style.weight,
        &style.font_style,
        style.font_stretch,
        ' ',
    )?;
    let shift = match style.vertical_align.as_str() {
        "super" | "top" => style.font_size * 0.35,
        "sub" | "bottom" => -style.font_size * 0.2,
        "middle" => style.font_size * 0.1,
        _ => 0.0,
    };
    let bbox_ratio = (font.bbox.y_max as f32 - font.bbox.y_min as f32) / font.units_per_em as f32;
    let use_font_bbox =
        use_font_bbox && style.use_font_bbox_for_line_height && (1.25..=1.35).contains(&bbox_ratio);
    Ok(InlineStrut {
        ascent: if use_font_bbox {
            font.bbox.y_max as f32 / font.units_per_em as f32 * style.font_size + shift
        } else {
            font.ascent as f32 / font.units_per_em as f32 * style.font_size + shift
        },
        descent: if use_font_bbox {
            -(font.bbox.y_min as f32 / font.units_per_em as f32 * style.font_size) - shift
        } else {
            -(font.descent as f32 / font.units_per_em as f32 * style.font_size) - shift
        },
        height: style.line_height,
    })
}

fn apply_inline_struts(
    units: &mut [ShapedUnit],
    start: usize,
    styles: &[Style],
    fonts: &FontRegistry,
) -> Result<()> {
    if styles.is_empty() || start >= units.len() {
        return Ok(());
    }
    let mut combined = InlineStrut {
        ascent: 0.0,
        descent: 0.0,
        height: 0.0,
    };
    for style in styles {
        let metrics = inline_strut(style, fonts, false)?;
        combined.ascent = combined.ascent.max(metrics.ascent);
        combined.descent = combined.descent.max(metrics.descent);
        combined.height = combined.height.max(metrics.height);
    }
    let combined = Arc::new(combined);
    for unit in &mut units[start..] {
        if let ShapedUnit::Glyph(glyph, _, _) = unit {
            glyph.inline_strut = Some(combined.clone());
            // A single inline run is already represented by its own line-height.
            // Only nested runs need the compatibility engine's extra font box.
            glyph.use_font_bbox_for_line_height &= styles.len() >= 2;
        }
    }
    Ok(())
}

fn inline_box_has_in_flow_content(node: &Node) -> bool {
    if node.tag == "#text" {
        return !node.text.trim().is_empty()
            || matches!(node.style.white_space.as_str(), "pre" | "pre-wrap");
    }
    if matches!(node.style.position.as_str(), "absolute" | "fixed") {
        return false;
    }
    matches!(node.tag.as_str(), "img" | "br" | "table")
        || node.children.iter().any(inline_box_has_in_flow_content)
}

#[allow(clippy::too_many_arguments)] // Kept explicit: these values are the inline layout context.
fn inline_box_glyph(
    node: &Node,
    href: Option<Arc<str>>,
    destination: Option<String>,
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
    let has_content = inline_box_has_in_flow_content(&content_node);
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
    } else if !has_content {
        horizontal_edges
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
    if inner_width <= 0.0 && has_content {
        return Err(Error("inline-block has no usable content width".into()));
    }
    let lines = if has_content {
        lines_for(
            &content_node,
            inner_width,
            0.0,
            fonts,
            token,
            experimental_shaping,
            shaping_ns,
        )?
    } else {
        Vec::new()
    };
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
        line_height: style.line_height,
        use_font_bbox_for_line_height: style.use_font_bbox_for_line_height,
        color: style.color,
        alpha: style.color_alpha * style.opacity,
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
            destination: destination.or_else(|| {
                node.attr("id")
                    .or_else(|| (node.tag == "a").then(|| node.attr("name")).flatten())
                    .map(str::to_owned)
            }),
        })),
        inline_strut: None,
        destination: None,
        paint_late: style.position == "relative",
    })
}

fn anchor_glyph(destination: String, style: &Style, fonts: &FontRegistry) -> Result<Glyph> {
    let font = fonts.resolve_with_stretch(
        &style.family,
        style.weight,
        &style.font_style,
        style.font_stretch,
        ' ',
    )?;
    let (id, _) = font.glyph(' ')?;
    Ok(Glyph {
        ch: ' ',
        id,
        font,
        advance: 0.0,
        natural_advance: 0.0,
        x_offset: 0.0,
        y_offset: 0.0,
        unicode: Some(Arc::<str>::from("")),
        size: style.font_size,
        line_height: style.line_height,
        use_font_bbox_for_line_height: style.use_font_bbox_for_line_height,
        color: style.color,
        alpha: style.color_alpha * style.opacity,
        shift: 0.0,
        decoration: TextDecoration::None,
        href: None,
        preserve_space: true,
        visible: false,
        soft_hyphen: false,
        soft_hyphen_advance: 0.0,
        inline_box: None,
        inline_strut: None,
        destination: Some(destination),
        paint_late: style.position == "relative",
    })
}

fn lines_for(
    node: &Node,
    width: f32,
    first_indent: f32,
    fonts: &FontRegistry,
    token: Option<&AtomicBool>,
    _experimental_shaping: bool,
    shaping_ns: &AtomicU64,
) -> Result<Vec<Line>> {
    let start = Instant::now();
    let result = lines_for_shaped(node, width, first_indent, fonts, token);
    shaping_ns.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    result
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
                line_height: style.line_height,
                use_font_bbox_for_line_height: style.use_font_bbox_for_line_height,
                color: style.color,
                alpha: style.color_alpha * style.opacity,
                shift,
                decoration,
                href: href.clone(),
                preserve_space: false,
                visible: style.visibility == "visible",
                soft_hyphen: false,
                soft_hyphen_advance: 0.0,
                inline_box: None,
                inline_strut: None,
                destination: None,
                paint_late: style.position == "relative",
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
    inline_runs(node, None, None, &mut runs, true, &mut Vec::new());
    let mut units = Vec::new();
    let mut previous_space = false;
    let mut character_index = 0usize;
    let mut preceding_rtl = false;
    for run in runs {
        let run_start = units.len();
        let style = &run.style;
        let preserve = matches!(style.white_space.as_str(), "pre" | "pre-wrap");
        if let Some(inline) = &run.inline_box {
            let glyph = inline_box_glyph(
                inline,
                run.href.clone(),
                run.destination.clone(),
                width,
                fonts,
                token,
                true,
                &AtomicU64::new(0),
            )?;
            units.push(ShapedUnit::Glyph(glyph, true, false));
            apply_inline_struts(&mut units, run_start, &run.inline_struts, fonts)?;
            previous_space = false;
            preceding_rtl = false;
            continue;
        }
        let mut segment = String::new();
        let mut segment_font: Option<Arc<FontData>> = None;
        let mut segment_rtl: Option<bool> = None;
        let mut cluster_fonts: HashMap<String, Arc<FontData>> = HashMap::new();
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
                        line_height: style.line_height,
                        use_font_bbox_for_line_height: style.use_font_bbox_for_line_height,
                        color: style.color,
                        alpha: style.color_alpha * style.opacity,
                        shift: 0.0,
                        decoration: TextDecoration::None,
                        href: None,
                        preserve_space: false,
                        visible: false,
                        soft_hyphen: false,
                        soft_hyphen_advance: 0.0,
                        inline_box: None,
                        inline_strut: None,
                        destination: None,
                        paint_late: style.position == "relative",
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
                        line_height: style.line_height,
                        use_font_bbox_for_line_height: style.use_font_bbox_for_line_height,
                        color: style.color,
                        alpha: style.color_alpha * style.opacity,
                        shift: 0.0,
                        decoration: TextDecoration::None,
                        href: run.href.clone(),
                        preserve_space: preserve,
                        visible: style.visibility == "visible",
                        soft_hyphen: false,
                        soft_hyphen_advance: 0.0,
                        inline_box: None,
                        inline_strut: None,
                        destination: None,
                        paint_late: style.position == "relative",
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
            let font = if let Some(font) = cluster_fonts.get(cluster) {
                font.clone()
            } else {
                let resolved = if cluster.len() == ch.len_utf8() {
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
                cluster_fonts.insert(cluster.to_owned(), resolved.clone());
                resolved
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
        if let Some(destination) = run.destination {
            if let Some(glyph) = units[run_start..].iter_mut().find_map(|unit| match unit {
                ShapedUnit::Glyph(glyph, _, _) if glyph.id != 0 => Some(glyph),
                _ => None,
            }) {
                glyph.destination = Some(destination);
            } else {
                units.push(ShapedUnit::Glyph(
                    anchor_glyph(destination, style, fonts)?,
                    false,
                    false,
                ));
            }
        }
        apply_inline_struts(&mut units, run_start, &run.inline_struts, fonts)?;
    }
    let mut default_height = node.style.line_height.max(node.style.font_size);
    if node.style.use_font_bbox_for_line_height
        && node
            .style
            .family
            .iter()
            .any(|family| family.eq_ignore_ascii_case("Inter"))
    {
        let font = fonts.resolve_with_stretch(
            &node.style.family,
            node.style.weight,
            &node.style.font_style,
            node.style.font_stretch,
            ' ',
        )?;
        let bbox_ratio =
            (font.bbox.y_max as f32 - font.bbox.y_min as f32) / font.units_per_em as f32;
        if (1.25..=1.35).contains(&bbox_ratio) {
            default_height = default_height.max(bbox_ratio * node.style.font_size);
        }
    }
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
        let collapsible_space = glyph.ch == ' ' && !glyph.preserve_space;
        if !collapsible_space
            && current_width + glyph.advance > available + EPS
            && !current.is_empty()
            && can_wrap
        {
            match last_break_opportunity(&current) {
                Some((_, true)) => {
                    current_width = break_at_soft_hyphen(&mut current, &mut lines, default_height)
                        .unwrap_or(0.0);
                }
                Some((last_space, false)) => {
                    let trailing = current.split_off(last_space + 1);
                    current.pop();
                    push_line(&mut lines, std::mem::take(&mut current), default_height);
                    let trailing_width: f32 = trailing.iter().map(|item| item.advance).sum();
                    if break_word && trailing_width + glyph.advance > available + EPS {
                        // The last word is wider than a fresh line. Move its
                        // fitting prefix to the next line before breaking it
                        // at a grapheme boundary; otherwise the preferred
                        // whitespace break can strand an over-wide word.
                        push_line(&mut lines, trailing, default_height);
                        current_width = 0.0;
                    } else {
                        current = trailing;
                        current_width = trailing_width;
                    }
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
    if let Some((index, line)) = lines.iter().enumerate().find(|(index, line)| {
        line.width > width - if *index == 0 { first_indent } else { 0.0 } + EPS
    }) {
        let available = width - if index == 0 { first_indent } else { 0.0 };
        let text: String = line.glyphs.iter().map(|glyph| glyph.ch).collect();
        return Err(Error(format!(
            "text line exceeds available width ({:.3} > {:.3}): {text}",
            line.width, available
        )));
    }
    Ok(lines)
}

struct Flow {
    pages: Vec<Page>,
    outlines: Vec<OutlineEntry>,
    counter_scopes: Vec<HashMap<String, Vec<i32>>>,
    page_counters: Vec<HashMap<String, Vec<i32>>>,
    strings: HashMap<String, String>,
    page_strings: Vec<HashMap<String, String>>,
    running_elements: HashMap<String, String>,
    page_running_elements: Vec<HashMap<String, String>>,
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
    max_image_dpi: Option<f32>,
    shaping_ns: AtomicU64,
    iterations: usize,
    resource_bytes: usize,
    pending_break: bool,
    pending_break_side: Option<String>,
    frame: Option<(f32, f32)>,
    responsive_frame_insets: Option<(f32, f32)>,
    center_children: bool,
    paint_visible: bool,
    containing_blocks: Vec<ContainingBlock>,
    fixed_nodes: Vec<Node>,
    positioned_layers: Vec<PositionedLayer>,
    layer_order: usize,
    source: SourceContext,
}

#[derive(Clone, Copy)]
struct ContainingBlock {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    clip: bool,
}

struct PositionedLayer {
    page: usize,
    z_index: i32,
    order: usize,
    items: Vec<Item>,
}

struct ContainerGeometry {
    margins: [f32; 4],
    padding: [f32; 4],
    outer: f32,
    width: f32,
    x: f32,
}

fn resolved_dimension(fixed: Option<f32>, percent: Option<f32>, reference: f32) -> Option<f32> {
    fixed.or_else(|| percent.map(|value| reference * value))
}
fn resolved_edges(fixed: [f32; 4], percentages: [Option<f32>; 4], reference: f32) -> [f32; 4] {
    std::array::from_fn(|index| fixed[index] + percentages[index].unwrap_or(0.0) * reference)
}
fn collapse_margin(a: f32, b: f32) -> f32 {
    if a >= 0.0 && b >= 0.0 {
        a.max(b)
    } else if a <= 0.0 && b <= 0.0 {
        a.min(b)
    } else {
        a + b
    }
}
fn painted_rect(x: f32, y: f32, w: f32, h: f32, fill: Color, alpha: f32, radius: [f32; 4]) -> Item {
    if alpha >= 1.0 - EPS && radius.iter().all(|value| *value <= EPS) {
        Item::Rect {
            x,
            y,
            w,
            h,
            fill: Some(fill),
            stroke: None,
        }
    } else {
        Item::RoundedRect {
            x,
            y,
            w,
            h,
            fill,
            radius,
            alpha,
        }
    }
}
fn inline_flow_child(node: &Node) -> bool {
    matches!(
        node.tag.as_str(),
        "#text" | "span" | "strong" | "b" | "em" | "i" | "u" | "a" | "sup" | "sub" | "br"
    )
}
fn collapsible_node_margin(node: &Node, width: f32, top: bool) -> f32 {
    let edges = resolved_edges(node.style.margin, node.style.margin_percent, width);
    let side = if top { 0 } else { 2 };
    let mut value = edges[side];
    let padding = resolved_edges(node.style.padding, node.style.padding_percent, width);
    if padding[side] > EPS || node.style.border_widths[side] > EPS {
        return value;
    }
    let candidate = if top {
        node.children
            .iter()
            .find(|child| child.tag != "#text" || !child.text.trim().is_empty())
    } else {
        node.children
            .iter()
            .rev()
            .find(|child| child.tag != "#text" || !child.text.trim().is_empty())
    };
    if let Some(child) = candidate.filter(|child| !inline_flow_child(child)) {
        value = collapse_margin(value, collapsible_node_margin(child, width, top));
    }
    value
}

fn constrained_dimension(value: f32, minimum: Option<f32>, maximum: Option<f32>) -> f32 {
    // CSS sizing gives the minimum precedence when min > max.
    value
        .min(maximum.unwrap_or(f32::INFINITY))
        .max(minimum.unwrap_or(0.0))
}

fn affine_multiply(left: [f32; 6], right: [f32; 6]) -> [f32; 6] {
    [
        left[0] * right[0] + left[2] * right[1],
        left[1] * right[0] + left[3] * right[1],
        left[0] * right[2] + left[2] * right[3],
        left[1] * right[2] + left[3] * right[3],
        left[0] * right[4] + left[2] * right[5] + left[4],
        left[1] * right[4] + left[3] * right[5] + left[5],
    ]
}

fn resolve_grid_tracks(tracks: &[css::GridTrack], available: f32, gap: f32) -> Result<Vec<f32>> {
    if tracks.is_empty() {
        return Ok(vec![available]);
    }
    let usable = available - gap * tracks.len().saturating_sub(1) as f32;
    if usable <= EPS {
        return Err(Error("grid gaps leave no usable track width".into()));
    }
    fn minimum(track: &css::GridTrack, reference: f32) -> f32 {
        match track {
            css::GridTrack::Fixed(value) => *value,
            css::GridTrack::Percent(value) => reference * value,
            css::GridTrack::MinMax(minimum_track, _) => minimum(minimum_track, reference),
            css::GridTrack::Fr(_) | css::GridTrack::Auto => 0.0,
        }
    }
    fn flexible(track: &css::GridTrack) -> f32 {
        match track {
            css::GridTrack::Fr(value) => *value,
            css::GridTrack::Auto => 1.0,
            css::GridTrack::MinMax(_, maximum) => match maximum.as_ref() {
                css::GridTrack::Fr(value) => *value,
                css::GridTrack::Auto => 1.0,
                _ => 0.0,
            },
            _ => 0.0,
        }
    }
    let mut values: Vec<_> = tracks.iter().map(|track| minimum(track, usable)).collect();
    let fixed: f32 = values.iter().sum();
    let weight: f32 = tracks.iter().map(flexible).sum();
    let remaining = (usable - fixed).max(0.0);
    if weight > 0.0 {
        for (value, track) in values.iter_mut().zip(tracks) {
            *value += remaining * flexible(track) / weight;
        }
    }
    for (value, track) in values.iter_mut().zip(tracks) {
        if let css::GridTrack::MinMax(_, maximum) = track {
            let cap = match maximum.as_ref() {
                css::GridTrack::Fixed(value) => Some(*value),
                css::GridTrack::Percent(value) => Some(usable * value),
                _ => None,
            };
            if let Some(cap) = cap {
                *value = value.min(cap).max(minimum(track, usable));
            }
        }
    }
    if values.iter().any(|value| *value <= EPS) {
        return Err(Error("grid track has no usable size".into()));
    }
    Ok(values)
}

fn range_bounds(items: &[Item]) -> Option<(f32, f32, f32, f32)> {
    items.iter().filter_map(item_bounds).fold(
        None,
        |bounds: Option<(f32, f32, f32, f32)>, (x, y, w, h)| {
            Some(match bounds {
                None => (x, y, x + w, y + h),
                Some((left, top, right, bottom)) => {
                    (left.min(x), top.min(y), right.max(x + w), bottom.max(y + h))
                }
            })
        },
    )
}

fn stretch_box_item(item: &mut Item, top: f32, height: f32) {
    match item {
        Item::Rect { y, h, .. }
        | Item::RoundedRect { y, h, .. }
        | Item::Border { y, h, .. }
        | Item::BeginClip { y, h, .. }
            if (*y - top).abs() <= EPS && *h < height =>
        {
            *h = height;
        }
        _ => {}
    }
}

impl Flow {
    fn margin_content(
        raw: &str,
        page: usize,
        pages: usize,
        counters: &HashMap<String, Vec<i32>>,
        strings: &HashMap<String, String>,
        running: &HashMap<String, String>,
    ) -> String {
        let mut out = String::new();
        let mut rest = raw.trim();
        while !rest.is_empty() {
            rest = rest.trim_start();
            let function = ["counter", "counters", "string", "element"]
                .into_iter()
                .find_map(|name| {
                    let tail = rest.strip_prefix(name)?.strip_prefix('(')?;
                    let close = tail.find(')')?;
                    Some((name, &tail[..close], &tail[close + 1..]))
                });
            if let Some(("counter", argument, tail)) = function {
                let value = match argument.trim() {
                    "page" => Some(page.to_string()),
                    "pages" => Some(pages.to_string()),
                    name => counters
                        .get(name)
                        .and_then(|stack| stack.last())
                        .map(ToString::to_string),
                };
                if let Some(value) = value {
                    out.push_str(&value);
                }
                rest = tail;
            } else if let Some(("counters", argument, tail)) = function {
                let name = argument.split(',').next().unwrap_or("").trim();
                let separator = argument
                    .split_once(',')
                    .map(|(_, separator)| separator.trim().trim_matches(['\'', '"']))
                    .unwrap_or("");
                if let Some(stack) = counters.get(name) {
                    out.push_str(
                        &stack
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(separator),
                    );
                }
                rest = tail;
            } else if let Some(("string", argument, tail)) = function {
                if let Some(value) = strings.get(argument.trim()) {
                    out.push_str(value);
                }
                rest = tail;
            } else if let Some(("element", argument, tail)) = function {
                if let Some(value) = running.get(argument.trim()) {
                    out.push_str(value);
                }
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
        let content = Self::margin_content(
            &box_style.content,
            index + 1,
            total,
            &self.page_counters[index],
            &self.page_strings[index],
            &self.page_running_elements[index],
        );
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
            source_offset: None,
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
            // Margin boxes are bounded regions. Use conservative glyph extents
            // for alignment because kerning may make the shaped pen advance
            // smaller than the final outline's nominal advance.
            let visual_width = line
                .glyphs
                .iter()
                .map(|glyph| {
                    glyph
                        .font
                        .glyph(glyph.ch)
                        .map(|(_, advance)| {
                            advance as f32 * glyph.size / glyph.font.units_per_em as f32
                        })
                        .unwrap_or(glyph.advance)
                })
                .sum::<f32>()
                .max(line.width);
            let offset = match align {
                "right" => (w - visual_width).max(0.0),
                "center" => ((w - visual_width) / 2.0).max(0.0),
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
            let counters = &self.page_counters[index];
            let strings = &self.page_strings[index];
            let running = &self.page_running_elements[index];
            let mut boxes: Vec<_> = self.pages[index].style.boxes.clone().into_iter().collect();
            boxes.sort_by(|left, right| left.0.cmp(&right.0));
            let occupied = |edge: &str| {
                ["left", "center", "right"].map(|side| {
                    boxes.iter().any(|(name, box_style)| {
                        name == &format!("@{edge}-{side}")
                            && !Self::margin_content(
                                &box_style.content,
                                index + 1,
                                total,
                                counters,
                                strings,
                                running,
                            )
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
    #[allow(clippy::too_many_arguments)]
    fn new(
        sheet: Sheet,
        fonts: FontRegistry,
        base_dir: PathBuf,
        images: Arc<Mutex<ImageCache>>,
        warnings: Vec<Diagnostic>,
        limits: RenderLimits,
        cancel: Option<Arc<AtomicBool>>,
        source: SourceContext,
    ) -> Self {
        let mut page = sheet.page_style_for(None, 0, false);
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
            outlines: Vec::new(),
            counter_scopes: vec![HashMap::new()],
            page_counters: vec![HashMap::new()],
            strings: HashMap::new(),
            page_strings: vec![HashMap::new()],
            running_elements: HashMap::new(),
            page_running_elements: vec![HashMap::new()],
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
            max_image_dpi: None,
            shaping_ns: AtomicU64::new(0),
            iterations: 0,
            resource_bytes: 0,
            pending_break: false,
            pending_break_side: None,
            frame: None,
            responsive_frame_insets: None,
            center_children: false,
            paint_visible: true,
            containing_blocks: vec![],
            fixed_nodes: vec![],
            positioned_layers: vec![],
            layer_order: 0,
            source,
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
    fn sync_page_frame(&mut self) {
        if let Some((left, right)) = self.responsive_frame_insets {
            self.frame = Some((
                self.page.margin[3] + left,
                self.page.width - self.page.margin[1] - self.page.margin[3] - left - right,
            ));
        }
    }
    fn new_page(&mut self) -> Result<()> {
        cancelled(self.cancel.as_deref())?;
        if self.pages.len() >= self.limits.max_pages {
            return Err(Error("render limit exceeded: max_pages".into()));
        }
        self.page = self.style_for(self.page_name.as_deref(), self.pages.len(), false);
        self.sync_page_frame();
        self.pages.push(Page {
            items: vec![],
            style: self.page.clone(),
            name: self.page_name.clone(),
        });
        self.page_counters.push(self.counter_values());
        self.page_strings.push(self.strings.clone());
        self.page_running_elements
            .push(self.running_elements.clone());
        self.page_content = false;
        self.y = self.page.margin[0];
        Ok(())
    }
    fn style_for(&self, name: Option<&str>, index: usize, blank: bool) -> PageStyle {
        let mut style = self.sheet.page_style_for(name, index, blank);
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
            self.sync_page_frame();
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
    fn defer_inline_paint(&mut self, item_start: usize) {
        let page_index = self.pages.len() - 1;
        let items = self
            .pages
            .last_mut()
            .map(|page| page.items.drain(item_start..).collect::<Vec<_>>())
            .unwrap_or_default();
        if items.is_empty() {
            return;
        }
        let order = self.layer_order;
        self.layer_order += 1;
        self.positioned_layers.push(PositionedLayer {
            page: page_index,
            z_index: 0,
            order,
            items,
        });
    }
    fn destination(&mut self, node: &Node, x: f32, y: f32) {
        let name = node
            .attr("id")
            .or_else(|| (node.tag == "a").then(|| node.attr("name")).flatten())
            .filter(|name| !name.is_empty());
        if let Some(name) = name {
            self.item(Item::Destination {
                name: name.to_owned(),
                x,
                y,
            });
        }
    }
    fn node(&mut self, node: &Node) -> Result<()> {
        let result = self.node_inner(node);
        result.map_err(|error| add_node_location(error, node.source_offset, &self.source))
    }
    fn node_inner(&mut self, node: &Node) -> Result<()> {
        cancelled(self.cancel.as_deref())?;
        self.iterations = self.iterations.saturating_add(1);
        if self.iterations > self.limits.max_layout_iterations {
            return Err(Error("render limit exceeded: max_layout_iterations".into()));
        }
        if let Some(name) = &node.style.running_name {
            let value = node
                .plain_text()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            self.running_elements.insert(name.clone(), value.clone());
            if let Some(page_values) = self.page_running_elements.last_mut() {
                page_values.insert(name.clone(), value);
            }
            return Ok(());
        }
        let mut reset_scope = HashMap::new();
        for (name, value) in &node.style.counter_reset {
            reset_scope.insert(name.clone(), vec![*value]);
        }
        let has_counter_scope = !reset_scope.is_empty();
        if has_counter_scope {
            self.counter_scopes.push(reset_scope);
        }
        for (name, amount) in &node.style.counter_increment {
            if let Some(counter) = self
                .counter_scopes
                .iter_mut()
                .rev()
                .find_map(|scope| scope.get_mut(name))
            {
                if let Some(value) = counter.last_mut() {
                    *value = value.saturating_add(*amount);
                }
            } else {
                self.counter_scopes
                    .last_mut()
                    .expect("root counter scope")
                    .insert(name.clone(), vec![amount.saturating_add(0)]);
            }
        }
        if let Some(name) = &node.style.string_set {
            self.strings.insert(
                name.clone(),
                node.plain_text()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        self.sync_page_context();
        let old_visibility = self.paint_visible;
        self.paint_visible = node.style.visibility == "visible";
        let result = self.positioned_node(node);
        self.paint_visible = old_visibility;
        if has_counter_scope {
            self.counter_scopes.pop();
        }
        result
    }
    fn counter_values(&self) -> HashMap<String, Vec<i32>> {
        let mut values = HashMap::new();
        for scope in &self.counter_scopes {
            for (name, stack) in scope {
                values
                    .entry(name.clone())
                    .and_modify(|current: &mut Vec<i32>| current.extend(stack.iter().copied()))
                    .or_insert_with(|| stack.clone());
            }
        }
        values
    }
    fn sync_page_context(&mut self) {
        let counters = self.counter_values();
        if let Some(values) = self.page_counters.last_mut() {
            *values = counters;
        }
        if let Some(values) = self.page_strings.last_mut() {
            *values = self.strings.clone();
        }
        if let Some(values) = self.page_running_elements.last_mut() {
            *values = self.running_elements.clone();
        }
    }
    fn positioned_node(&mut self, node: &Node) -> Result<()> {
        if node.style.position == "fixed" {
            self.fixed_nodes.push(node.clone());
            return Ok(());
        }
        if node.style.position == "absolute" {
            return self.out_of_flow_node(node, false);
        }
        let creates_stacking_context = node.style.position == "relative"
            || node.style.transform != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let stacking_order = creates_stacking_context.then(|| {
            let order = self.layer_order;
            self.layer_order += 1;
            order
        });
        let auto_block_transform_box = (node.style.display == "block"
            && node.style.width.is_none()
            && node.style.width_percent.is_none())
        .then(|| {
            let x = self.content_x();
            let width = self.content_width();
            let margins = resolved_edges(node.style.margin, node.style.margin_percent, width);
            (
                x + margins[3],
                x + width - margins[1],
                self.pages.len() - 1,
                self.y + margins[0],
                margins[2],
                self.pages.len(),
            )
        });
        let item_counts: Vec<usize> = self.pages.iter().map(|p| p.items.len()).collect();
        self.node_content(node)?;
        let auto_block_transform_y = auto_block_transform_box.and_then(
            |(_, _, page_index, top, bottom_margin, initial_page_count)| {
                (self.pages.len() == initial_page_count).then_some((
                    page_index,
                    top,
                    self.y - bottom_margin,
                ))
            },
        );
        if node.style.position == "relative" {
            let horizontal = |index: usize| {
                node.style.inset[index].or_else(|| {
                    node.style.inset_percent[index].map(|value| self.content_width() * value)
                })
            };
            let vertical = |index: usize| {
                node.style.inset[index].or_else(|| {
                    node.style.inset_percent[index].map(|value| self.full_height() * value)
                })
            };
            let dx = horizontal(3)
                .or_else(|| horizontal(1).map(|right| -right))
                .unwrap_or(0.0);
            let dy = vertical(0)
                .or_else(|| vertical(2).map(|bottom| -bottom))
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
        }
        if node.style.transform != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] {
            for (index, page) in self.pages.iter_mut().enumerate() {
                let start = *item_counts.get(index).unwrap_or(&0);
                if start >= page.items.len() {
                    continue;
                }
                let mut bounds = page.items[start..].iter().filter_map(item_bounds).fold(
                    None,
                    |bounds: Option<(f32, f32, f32, f32)>, (x, y, w, h)| {
                        Some(match bounds {
                            None => (x, y, x + w, y + h),
                            Some((left, top, right, bottom)) => {
                                (left.min(x), top.min(y), right.max(x + w), bottom.max(y + h))
                            }
                        })
                    },
                );
                if let (Some((box_left, box_right, _, _, _, _)), Some((left, top, right, bottom))) =
                    (auto_block_transform_box, bounds)
                {
                    bounds = Some((left.min(box_left), top, right.max(box_right), bottom));
                }
                if let (Some((box_page, box_top, box_bottom)), Some((left, top, right, bottom))) =
                    (auto_block_transform_y, bounds)
                {
                    if index == box_page {
                        bounds = Some((left, top.min(box_top), right, bottom.max(box_bottom)));
                    }
                }
                if let Some((left, top, right, bottom)) = bounds {
                    let origin_x = (left + right) / 2.0;
                    let origin_y = (top + bottom) / 2.0;
                    let matrix = affine_multiply(
                        [1.0, 0.0, 0.0, 1.0, origin_x, origin_y],
                        affine_multiply(
                            node.style.transform,
                            [1.0, 0.0, 0.0, 1.0, -origin_x, -origin_y],
                        ),
                    );
                    page.items.insert(start, Item::BeginTransform(matrix));
                    page.items.push(Item::EndTransform);
                }
            }
        }
        if node.style.position == "relative" || creates_stacking_context {
            let z_index = if node.style.position == "relative" {
                node.style.z_index
            } else {
                0
            };
            for (page_index, page) in self.pages.iter_mut().enumerate() {
                let start = *item_counts.get(page_index).unwrap_or(&0);
                if start >= page.items.len() {
                    continue;
                }
                self.positioned_layers.push(PositionedLayer {
                    page: page_index,
                    z_index,
                    order: stacking_order.expect("stacking context has an order"),
                    items: page.items.split_off(start),
                });
            }
        }
        Ok(())
    }
    fn current_containing_block(&self, fixed: bool) -> ContainingBlock {
        if !fixed {
            if let Some(block) = self.containing_blocks.last() {
                return *block;
            }
        }
        ContainingBlock {
            x: self.page.margin[3],
            y: self.page.margin[0],
            w: self.page.width - self.page.margin[1] - self.page.margin[3],
            h: self.page.height - self.page.margin[0] - self.page.margin[2],
            clip: false,
        }
    }
    fn out_of_flow_node(&mut self, node: &Node, fixed: bool) -> Result<()> {
        let block = self.current_containing_block(fixed);
        let inset = [
            node.style.inset[0]
                .or_else(|| node.style.inset_percent[0].map(|value| block.h * value)),
            node.style.inset[1]
                .or_else(|| node.style.inset_percent[1].map(|value| block.w * value)),
            node.style.inset[2]
                .or_else(|| node.style.inset_percent[2].map(|value| block.h * value)),
            node.style.inset[3]
                .or_else(|| node.style.inset_percent[3].map(|value| block.w * value)),
        ];
        let horizontal = node.style.border_widths[1]
            + node.style.border_widths[3]
            + node.style.padding[1]
            + node.style.padding[3];
        let declared_width =
            resolved_dimension(node.style.width, node.style.width_percent, block.w).map(|width| {
                if node.style.box_sizing == "border-box" {
                    width
                } else {
                    width + horizontal
                }
            });
        let width = declared_width
            .or_else(|| match (inset[3], inset[1]) {
                (Some(left), Some(right)) => Some((block.w - left - right).max(EPS)),
                (Some(left), None) => Some((block.w - left).max(EPS)),
                (None, Some(right)) => Some((block.w - right).max(EPS)),
                _ => None,
            })
            .unwrap_or(block.w);
        let x = if let Some(left) = inset[3] {
            block.x + left
        } else if let Some(right) = inset[1] {
            block.x + block.w - right - width
        } else {
            block.x
        };
        let declared_height =
            resolved_dimension(node.style.height, node.style.height_percent, block.h);
        let height = declared_height.or_else(|| match (inset[0], inset[2]) {
            (Some(top), Some(bottom)) => Some((block.h - top - bottom).max(EPS)),
            _ => None,
        });
        let y = if let Some(top) = inset[0] {
            block.y + top
        } else if let (Some(bottom), Some(height)) = (inset[2], height) {
            block.y + block.h - bottom - height
        } else {
            block.y
        };

        let page_index = self.pages.len() - 1;
        let start = self.pages[page_index].items.len();
        let saved_y = self.y;
        let saved_frame = self.frame;
        let saved_page_content = self.page_content;
        let saved_pending = self.pending_break;
        let saved_pending_side = self.pending_break_side.clone();
        self.y = y;
        self.frame = Some((x, width));
        self.page_content = false;
        let mut clone = node.clone();
        // Keep a positioning context for nested absolute descendants without
        // applying the out-of-flow algorithm recursively to this box itself.
        clone.style.position = "relative".into();
        clone.style.inset = [None; 4];
        clone.style.inset_percent = [None; 4];
        clone.style.margin = [0.0; 4];
        clone.style.margin_percent = [None; 4];
        clone.style.width = Some(width);
        clone.style.width_percent = None;
        clone.style.box_sizing = "border-box".into();
        clone.style.break_before = false;
        clone.style.break_after = false;
        clone.style.break_before_side = None;
        clone.style.break_after_side = None;
        clone.style.page_name = None;
        if let Some(height) = height {
            clone.style.height = Some(height);
            clone.style.height_percent = None;
        }
        self.node(&clone)?;
        if self.pages.len() - 1 != page_index {
            return Err(Error(if fixed {
                "fixed positioned box cannot fragment across pages".into()
            } else {
                "absolute positioned box cannot fragment across pages".into()
            }));
        }
        let mut items = self.pages[page_index].items.split_off(start);
        if block.clip && !items.is_empty() {
            items.insert(
                0,
                Item::BeginClip {
                    x: block.x,
                    y: block.y,
                    w: block.w,
                    h: block.h,
                    radius: [0.0; 4],
                },
            );
            items.push(Item::EndClip);
        }
        self.positioned_layers.push(PositionedLayer {
            page: page_index,
            z_index: node.style.z_index,
            order: self.layer_order,
            items,
        });
        self.layer_order += 1;
        self.y = saved_y;
        self.frame = saved_frame;
        self.page_content = saved_page_content;
        self.pending_break = saved_pending;
        self.pending_break_side = saved_pending_side;
        Ok(())
    }
    fn render_fixed_nodes(&mut self) -> Result<()> {
        if self.fixed_nodes.is_empty() {
            return Ok(());
        }
        let nodes = std::mem::take(&mut self.fixed_nodes);
        let page_count = self.pages.len();
        let last = page_count - 1;
        let saved_page = self.page.clone();
        let saved_name = self.page_name.clone();
        let saved_y = self.y;
        let saved_frame = self.frame;
        let saved_content = self.page_content;
        for page_index in 0..page_count {
            if page_index != last {
                self.pages.swap(page_index, last);
            }
            self.page = self.pages[last].style.clone();
            self.page_name = self.pages[last].name.clone();
            self.y = self.page.margin[0];
            self.frame = None;
            let first_layer = self.positioned_layers.len();
            for node in &nodes {
                self.out_of_flow_node(node, true)?;
            }
            for layer in &mut self.positioned_layers[first_layer..] {
                layer.page = page_index;
            }
            if page_index != last {
                self.pages.swap(page_index, last);
            }
        }
        self.page = saved_page;
        self.page_name = saved_name;
        self.y = saved_y;
        self.frame = saved_frame;
        self.page_content = saved_content;
        Ok(())
    }
    fn compose_positioned_layers(&mut self) {
        for page_index in 0..self.pages.len() {
            let mut layers: Vec<_> = self
                .positioned_layers
                .iter_mut()
                .filter(|layer| layer.page == page_index)
                .collect();
            layers.sort_by_key(|layer| (layer.z_index, layer.order));
            let normal = std::mem::take(&mut self.pages[page_index].items);
            let mut items = Vec::new();
            for layer in layers.iter_mut().filter(|layer| layer.z_index < 0) {
                items.append(&mut layer.items);
            }
            items.extend(normal);
            for layer in layers.iter_mut().filter(|layer| layer.z_index >= 0) {
                items.append(&mut layer.items);
            }
            self.pages[page_index].items = items;
        }
        self.positioned_layers.clear();
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
            return self.grid_advanced(node);
        }
        if node.style.display == "flex" {
            return self.flex_advanced(node);
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
        self.block_container(node)
    }
    fn block_container(&mut self, node: &Node) -> Result<()> {
        self.begin(&node.style)?;
        let style = &node.style;
        let containing_width = self.content_width();
        let margins = resolved_edges(style.margin, style.margin_percent, containing_width);
        let padding = resolved_edges(style.padding, style.padding_percent, containing_width);
        let borders = style.border_widths;
        let horizontal_edges = padding[1] + padding[3] + borders[1] + borders[3];
        let available = containing_width - margins[1] - margins[3];
        let sizing = constrained_dimension(
            resolved_dimension(style.width, style.width_percent, containing_width).unwrap_or(
                if style.box_sizing == "border-box" {
                    available
                } else {
                    available - horizontal_edges
                },
            ),
            resolved_dimension(style.min_width, style.min_width_percent, containing_width),
            resolved_dimension(style.max_width, style.max_width_percent, containing_width),
        );
        let outer_width = if style.box_sizing == "border-box" {
            sizing
        } else {
            sizing + horizontal_edges
        };
        let inner_width = outer_width - horizontal_edges;
        if inner_width <= EPS {
            return Err(Error("block container has no usable width".into()));
        }
        let first_block = node
            .children
            .iter()
            .find(|child| {
                (child.tag == "#text"
                    || (child.style.position != "absolute" && child.style.position != "fixed"))
                    && (child.tag != "#text" || !child.text.trim().is_empty())
            })
            .filter(|child| {
                !matches!(child.style.display.as_str(), "inline" | "inline-block")
                    && !matches!(
                        child.tag.as_str(),
                        "#text"
                            | "span"
                            | "strong"
                            | "b"
                            | "em"
                            | "i"
                            | "u"
                            | "a"
                            | "sup"
                            | "sub"
                            | "br"
                    )
            });
        let collapse_first = !style.margin_top_consumed
            && padding[0] <= EPS
            && borders[0] <= EPS
            && first_block.is_some();
        let first_top = first_block
            .map(|child| collapsible_node_margin(child, inner_width, true))
            .unwrap_or(0.0);
        self.y += if collapse_first {
            collapse_margin(margins[0], first_top)
        } else {
            margins[0]
        };
        let box_y = self.y;
        self.destination(node, self.content_x() + margins[3], box_y);
        let page_index = self.pages.len() - 1;
        let insert_at = self.pages[page_index].items.len();
        self.y += borders[0] + padding[0];
        let content_start = self.y;
        let old_frame = self.frame;
        let old_responsive_frame_insets = self.responsive_frame_insets;
        self.frame = Some((
            self.content_x() + margins[3] + borders[3] + padding[3],
            inner_width,
        ));
        self.responsive_frame_insets = if style.width.is_none()
            && style
                .width_percent
                .is_none_or(|percent| (percent - 1.0).abs() <= EPS)
        {
            let (parent_left, parent_right) = old_responsive_frame_insets.unwrap_or((0.0, 0.0));
            Some((
                parent_left + margins[3] + borders[3] + padding[3],
                parent_right + margins[1] + borders[1] + padding[1],
            ))
        } else {
            None
        };
        let establishes_containing_block = style.position != "static";
        if establishes_containing_block {
            self.containing_blocks.push(ContainingBlock {
                x: self.frame.expect("block frame").0,
                y: content_start,
                w: inner_width,
                h: resolved_dimension(style.height, style.height_percent, self.full_height())
                    .unwrap_or_else(|| (self.limit() - content_start).max(EPS)),
                clip: style.overflow == "hidden",
            });
        }
        let mut group = Vec::new();
        let mut pending_margin = 0.0;
        let mut first_block_pending = style.margin_top_consumed || collapse_first;
        let children = node
            .children
            .iter()
            .flat_map(Self::split_inline_blocks)
            .collect::<Vec<_>>();
        let inline_only = children
            .iter()
            .all(|child| child.tag == "#text" || child.style.display == "inline");
        if style.column_count > 1 && inline_only {
            let mut inline = node.clone();
            inline.children = children;
            inline.style.margin = [0.0; 4];
            inline.style.margin_percent = [None; 4];
            inline.style.padding = [0.0; 4];
            inline.style.padding_percent = [None; 4];
            inline.style.border_width = 0.0;
            inline.style.border_widths = [0.0; 4];
            inline.style.background = None;
            inline.style.width = None;
            inline.style.width_percent = None;
            inline.style.break_before = false;
            inline.style.break_after = false;
            self.inline_columns(&inline, inner_width, content_start)?;
        } else {
            if style.column_count > 1 {
                self.warnings.push(
                    Diagnostic::new(
                        "css-property",
                        "column-count is supported only for inline-only block content",
                    )
                    .property("column-count", &style.column_count.to_string()),
                );
            }
            for child in &children {
                if child.tag != "#text"
                    && matches!(child.style.position.as_str(), "absolute" | "fixed")
                {
                    self.node(child)?;
                    continue;
                }
                if matches!(child.style.display.as_str(), "inline" | "inline-block")
                    || matches!(
                        child.tag.as_str(),
                        "#text"
                            | "span"
                            | "strong"
                            | "b"
                            | "em"
                            | "i"
                            | "u"
                            | "a"
                            | "sup"
                            | "sub"
                            | "br"
                    )
                {
                    group.push(child.clone());
                } else {
                    if group.iter().any(inline_box_has_in_flow_content) {
                        let mut inline = node.clone();
                        inline.children = std::mem::take(&mut group);
                        inline.style.margin = [0.0; 4];
                        inline.style.margin_percent = [None; 4];
                        inline.style.padding = [0.0; 4];
                        inline.style.padding_percent = [None; 4];
                        inline.style.border_width = 0.0;
                        inline.style.border_widths = [0.0; 4];
                        inline.style.background = None;
                        inline.style.width = None;
                        inline.style.width_percent = None;
                        inline.style.break_before = false;
                        inline.style.break_after = false;
                        self.paragraph(&inline)?;
                    }
                    group.clear();
                    let raw_child_margins =
                        resolved_edges(child.style.margin, child.style.margin_percent, inner_width);
                    let child_margins = [
                        collapsible_node_margin(child, inner_width, true),
                        raw_child_margins[1],
                        collapsible_node_margin(child, inner_width, false),
                        raw_child_margins[3],
                    ];
                    let child_top = if first_block_pending {
                        first_block_pending = false;
                        0.0
                    } else {
                        child_margins[0]
                    };
                    self.y += collapse_margin(pending_margin, child_top);
                    let mut child = child.clone();
                    child.style.margin[0] = 0.0;
                    child.style.margin[2] = 0.0;
                    child.style.margin_percent[0] = None;
                    child.style.margin_percent[2] = None;
                    child.style.margin_top_consumed = true;
                    child.style.margin_bottom_consumed = true;
                    self.node(&child)?;
                    if child.tag == "img" && style.use_font_bbox_for_line_height {
                        let font = self.fonts.resolve_with_stretch(
                            &style.family,
                            style.weight,
                            &style.font_style,
                            style.font_stretch,
                            ' ',
                        )?;
                        self.y +=
                            font.win_descent as f32 / font.units_per_em as f32 * style.font_size;
                    }
                    pending_margin = child_margins[2];
                }
            }
            if group.iter().any(inline_box_has_in_flow_content) {
                let mut inline = node.clone();
                inline.children = group;
                inline.style.margin = [0.0; 4];
                inline.style.margin_percent = [None; 4];
                inline.style.padding = [0.0; 4];
                inline.style.padding_percent = [None; 4];
                inline.style.border_width = 0.0;
                inline.style.border_widths = [0.0; 4];
                inline.style.background = None;
                inline.style.width = None;
                inline.style.width_percent = None;
                inline.style.break_before = false;
                inline.style.break_after = false;
                self.paragraph(&inline)?;
            }
        }
        if establishes_containing_block {
            self.containing_blocks.pop();
        }
        let collapse_last = !style.margin_bottom_consumed
            && padding[2] <= EPS
            && borders[2] <= EPS
            && style.height.is_none()
            && style.height_percent.is_none()
            && style.min_height.is_none()
            && style.min_height_percent.is_none();
        let outer_bottom = if style.margin_bottom_consumed {
            0.0
        } else if collapse_last {
            collapse_margin(pending_margin, margins[2])
        } else {
            self.y += pending_margin;
            margins[2]
        };
        self.frame = old_frame;
        self.responsive_frame_insets = old_responsive_frame_insets;
        let natural_content = (self.y - content_start).max(0.0);
        let vertical_edges = borders[0] + borders[2] + padding[0] + padding[2];
        let natural_height = natural_content + vertical_edges;
        let reference = self.full_height();
        let requested = constrained_dimension(
            resolved_dimension(style.height, style.height_percent, reference).unwrap_or(
                if style.box_sizing == "border-box" {
                    natural_height
                } else {
                    natural_content
                },
            ),
            resolved_dimension(style.min_height, style.min_height_percent, reference),
            resolved_dimension(style.max_height, style.max_height_percent, reference),
        );
        let requested_outer = if style.box_sizing == "border-box" {
            requested
        } else {
            requested + vertical_edges
        };
        let box_height = if style.overflow == "hidden" {
            requested_outer
        } else {
            natural_height.max(requested_outer)
        };
        let fragmented = self.pages.len() - 1 != page_index;
        self.y = if fragmented {
            // Children may have moved the flow cursor to a later page. Their
            // final-page position is already absolute within that page, so do
            // not add the container's first-page origin a second time.
            self.y + borders[2] + padding[2] + outer_bottom
        } else {
            box_y + box_height + outer_bottom
        };
        if fragmented {
            if style.overflow == "hidden" {
                return Err(Error(
                    "overflow:hidden block cannot fragment across pages".into(),
                ));
            }
        } else if style.visibility == "visible" {
            let page = &mut self.pages[page_index];
            let x = self.frame.map_or(self.page.margin[3], |frame| frame.0) + margins[3];
            let mut prefix = Vec::new();
            if let Some(fill) = style.background {
                prefix.push(painted_rect(
                    x,
                    box_y,
                    outer_width,
                    box_height,
                    fill,
                    style.background_alpha * style.opacity,
                    style.border_radius,
                ));
            }
            if style.border_widths.iter().any(|v| *v > 0.0) {
                prefix.push(Item::Border {
                    x,
                    y: box_y,
                    w: outer_width,
                    h: box_height,
                    widths: style.border_widths,
                    colors: style.border_colors,
                    alphas: style.border_alphas.map(|a| a * style.opacity),
                    styles: style.border_styles.clone(),
                    radius: style.border_radius,
                });
            }
            if style.overflow == "hidden" {
                prefix.push(Item::BeginClip {
                    x,
                    y: box_y,
                    w: outer_width,
                    h: box_height,
                    radius: style.border_radius,
                });
            }
            for (offset, item) in prefix.into_iter().enumerate() {
                page.items.insert(insert_at + offset, item);
            }
            if style.overflow == "hidden" {
                page.items.push(Item::EndClip);
            }
        }
        self.finish(&node.style);
        Ok(())
    }
    fn inline_columns(&mut self, node: &Node, width: f32, start_y: f32) -> Result<()> {
        let count = node.style.column_count;
        let gap = node.style.column_gap;
        let column_width = (width - gap * count.saturating_sub(1) as f32) / count as f32;
        if column_width <= 0.0 {
            return Err(Error("column gap leaves no usable column width".into()));
        }
        let lines = lines_for(
            node,
            column_width,
            node.style.text_indent,
            &self.fonts,
            self.cancel.as_deref(),
            self.experimental_shaping,
            &self.shaping_ns,
        )?;
        if lines.is_empty() {
            return Ok(());
        }
        let lines_per_column = lines.len().div_ceil(count);
        let x = self.content_x();
        let mut start_y = start_y;
        let largest_column_height = (0..count)
            .map(|column| {
                let first = column * lines_per_column;
                let last = (first + lines_per_column).min(lines.len());
                lines[first..last]
                    .iter()
                    .map(|line| line.height)
                    .sum::<f32>()
            })
            .fold(0.0f32, f32::max);
        if largest_column_height <= self.full_height() + EPS
            && start_y + largest_column_height > self.limit() + EPS
            && self.page_has_content()
        {
            self.new_page()?;
            start_y = self.page.margin[0];
        }
        let mut final_y = start_y;
        for column in 0..count {
            let first = column * lines_per_column;
            let last = (first + lines_per_column).min(lines.len());
            if first == last {
                break;
            }
            let mut y = start_y;
            for (line_index, line) in lines[first..last].iter().enumerate() {
                if y + line.height > self.limit() + EPS {
                    if self.page_has_content() {
                        self.new_page()?;
                        start_y = self.page.margin[0];
                        y = start_y;
                    } else {
                        return Err(Error(
                            "multi-column text exceeds page content height".into(),
                        ));
                    }
                }
                self.draw_line(
                    line,
                    x + column as f32 * (column_width + gap),
                    y,
                    column_width,
                    &node.style.text_align,
                    line_index + 1 == last - first,
                );
                y += line.height;
            }
            final_y = y;
        }
        self.y = final_y;
        Ok(())
    }
    fn split_inline_blocks(node: &Node) -> Vec<Node> {
        if node.style.display != "inline" || !Self::has_block_descendant(node) {
            return vec![node.clone()];
        }
        let mut result = Vec::new();
        let mut inline_segment = node.clone();
        inline_segment.children.clear();
        for child in &node.children {
            let pieces = if child.style.display == "inline" && Self::has_block_descendant(child) {
                Self::split_inline_blocks(child)
            } else {
                vec![child.clone()]
            };
            for piece in pieces {
                if piece.tag == "#text" || piece.style.display == "inline" {
                    inline_segment.children.push(piece);
                } else {
                    if !inline_segment.children.is_empty() {
                        result.push(inline_segment);
                        inline_segment = node.clone();
                        inline_segment.children.clear();
                    }
                    result.push(piece);
                }
            }
        }
        if !inline_segment.children.is_empty() {
            result.push(inline_segment);
        }
        if result
            .last()
            .is_some_and(|piece| piece.style.display != "inline")
        {
            result.push(Node {
                tag: "br".into(),
                attrs: Default::default(),
                style: node.style.clone(),
                text: String::new(),
                children: vec![],
                source_offset: node.source_offset,
            });
        }
        result
    }
    fn has_block_descendant(node: &Node) -> bool {
        node.children.iter().any(|child| {
            (child.tag != "#text" && child.style.display != "inline")
                || Self::has_block_descendant(child)
        })
    }
    fn container_geometry(&self, style: &Style, kind: &str) -> Result<ContainerGeometry> {
        let reference = self.content_width();
        let margins = resolved_edges(style.margin, style.margin_percent, reference);
        let padding = resolved_edges(style.padding, style.padding_percent, reference);
        let edges = padding[1] + padding[3] + style.border_widths[1] + style.border_widths[3];
        let available = reference - margins[1] - margins[3];
        let sizing = constrained_dimension(
            resolved_dimension(style.width, style.width_percent, reference).unwrap_or(
                if style.box_sizing == "border-box" {
                    available
                } else {
                    available - edges
                },
            ),
            resolved_dimension(style.min_width, style.min_width_percent, reference),
            resolved_dimension(style.max_width, style.max_width_percent, reference),
        );
        let outer = if style.box_sizing == "border-box" {
            sizing
        } else {
            sizing + edges
        };
        let width = outer - edges;
        if width <= EPS {
            return Err(Error(format!("{kind} container has no usable width")));
        }
        let x = self.content_x() + margins[3] + style.border_widths[3] + padding[3];
        Ok(ContainerGeometry {
            margins,
            padding,
            outer,
            width,
            x,
        })
    }

    fn flex_advanced(&mut self, node: &Node) -> Result<()> {
        self.begin(&node.style)?;
        let ContainerGeometry {
            margins,
            padding,
            outer: _outer,
            width,
            x,
        } = self.container_geometry(&node.style, "flex")?;
        self.y += margins[0] + node.style.border_widths[0] + padding[0];
        let start_y = self.y;
        let start_page = self.pages.len();
        let old_frame = self.frame;
        let declared_height = resolved_dimension(
            node.style.height,
            node.style.height_percent,
            self.full_height(),
        );
        let establishes = node.style.position != "static";
        if establishes {
            self.containing_blocks.push(ContainingBlock {
                x,
                y: start_y,
                w: width,
                h: declared_height.unwrap_or_else(|| (self.limit() - start_y).max(EPS)),
                clip: node.style.overflow == "hidden",
            });
        }
        for child in node.children.iter().filter(|child| {
            child.tag != "#text" && matches!(child.style.position.as_str(), "absolute" | "fixed")
        }) {
            self.node(child)?;
        }
        let mut children: Vec<_> = node
            .children
            .iter()
            .filter(|child| {
                (child.tag == "#text"
                    || !matches!(child.style.position.as_str(), "absolute" | "fixed"))
                    && (child.tag != "#text" || !child.text.trim().is_empty())
            })
            .collect();
        let reverse = node.style.flex_direction.ends_with("-reverse");
        if reverse {
            children.reverse();
        }
        let is_row = node.style.flex_direction.starts_with("row");
        if is_row {
            self.flex_rows(node, &children, x, width, start_y, declared_height)?;
        } else {
            self.flex_column(node, &children, x, width, start_y, declared_height)?;
        }
        if establishes {
            self.containing_blocks.pop();
        }
        self.frame = old_frame;
        let used = if self.pages.len() == start_page {
            (self.y - start_y).max(0.0)
        } else {
            0.0
        };
        let height = constrained_dimension(
            declared_height.unwrap_or(used),
            resolved_dimension(
                node.style.min_height,
                node.style.min_height_percent,
                self.full_height(),
            ),
            resolved_dimension(
                node.style.max_height,
                node.style.max_height_percent,
                self.full_height(),
            ),
        );
        let constrained = declared_height.is_some()
            || node.style.min_height.is_some()
            || node.style.min_height_percent.is_some()
            || node.style.max_height.is_some()
            || node.style.max_height_percent.is_some();
        if self.pages.len() != start_page && constrained {
            return Err(Error(
                "height-constrained flex container cannot split across pages".into(),
            ));
        }
        if self.pages.len() == start_page {
            self.y = start_y + used.max(height);
        }
        self.y += padding[2] + node.style.border_widths[2] + margins[2];
        self.finish(&node.style);
        Ok(())
    }

    fn flex_rows(
        &mut self,
        node: &Node,
        children: &[&Node],
        x: f32,
        width: f32,
        start_y: f32,
        declared_height: Option<f32>,
    ) -> Result<()> {
        if children.is_empty() {
            self.y = start_y;
            return Ok(());
        }
        let initial_page_count = self.pages.len();
        let fallback = ((width - node.style.column_gap * children.len().saturating_sub(1) as f32)
            / children.len() as f32)
            .max(EPS);
        let mut bases = Vec::with_capacity(children.len());
        for child in children {
            let explicit = resolved_dimension(
                child.style.flex_basis,
                child.style.flex_basis_percent,
                width,
            )
            .or_else(|| resolved_dimension(child.style.width, child.style.width_percent, width));
            let intrinsic = if explicit.is_none()
                && matches!(child.style.display.as_str(), "inline" | "inline-block")
            {
                let font = self.fonts.resolve_with_stretch(
                    &child.style.family,
                    child.style.weight,
                    &child.style.font_style,
                    child.style.font_stretch,
                    ' ',
                )?;
                let text_width = child
                    .plain_text()
                    .split('\n')
                    .map(|line| font.width(line, child.style.font_size))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .fold(0.0, f32::max);
                let inline_edges = if child.style.display == "inline-block" {
                    child.style.padding[1] + child.style.padding[3] + 2.0 * child.style.border_width
                } else {
                    0.0
                };
                text_width + inline_edges
            } else {
                0.0
            };
            bases.push(
                explicit
                    .unwrap_or(if intrinsic > EPS { intrinsic } else { fallback })
                    .max(EPS),
            );
        }
        let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
        let mut occupied = 0.0;
        for (index, basis) in bases.iter().enumerate() {
            let addition = if lines.last().is_some_and(|line| line.is_empty()) {
                *basis
            } else {
                node.style.column_gap + *basis
            };
            if node.style.flex_wrap != "nowrap"
                && occupied + addition > width + EPS
                && lines.last().is_some_and(|line| !line.is_empty())
            {
                lines.push(vec![index]);
                occupied = *basis;
            } else {
                lines.last_mut().unwrap().push(index);
                occupied += addition;
            }
        }
        if node.style.flex_wrap == "wrap-reverse" {
            lines.reverse();
        }
        let mut line_records: Vec<(usize, usize, f32, f32)> = Vec::new();
        self.y = start_y;
        for (line_number, line) in lines.iter().enumerate() {
            let estimate = line
                .iter()
                .filter_map(|index| {
                    resolved_dimension(
                        children[*index].style.height,
                        children[*index].style.height_percent,
                        declared_height.unwrap_or(self.full_height()),
                    )
                })
                .fold(40.0_f32, f32::max);
            if self.y + estimate > self.limit() + EPS && self.page_has_content() {
                self.new_page()?;
            }
            let row_page = self.pages.len();
            let row_y = self.y;
            let row_start = self.pages.last().map_or(0, |page| page.items.len());
            let base_total: f32 = line.iter().map(|index| bases[*index]).sum();
            let gaps = node.style.column_gap * line.len().saturating_sub(1) as f32;
            let free = width - base_total - gaps;
            let grow_total: f32 = line
                .iter()
                .map(|index| children[*index].style.flex_grow)
                .sum();
            let shrink_total: f32 = line
                .iter()
                .map(|index| children[*index].style.flex_shrink * bases[*index])
                .sum();
            let mut sizes: Vec<f32> = line
                .iter()
                .map(|index| {
                    let mut value = bases[*index];
                    if free > 0.0 && grow_total > 0.0 {
                        value += free * children[*index].style.flex_grow / grow_total;
                    } else if free < 0.0 && shrink_total > 0.0 {
                        value += free * children[*index].style.flex_shrink * bases[*index]
                            / shrink_total;
                    }
                    value.max(EPS)
                })
                .collect();
            let used_main: f32 = sizes.iter().sum::<f32>() + gaps;
            let remaining = (width - used_main).max(0.0);
            let (mut cursor, extra_gap) = match node.style.justify_content.as_str() {
                "end" => (x + remaining, 0.0),
                "center" => (x + remaining / 2.0, 0.0),
                "space-between" if line.len() > 1 => (x, remaining / (line.len() - 1) as f32),
                "space-around" => {
                    let gap = remaining / line.len() as f32;
                    (x + gap / 2.0, gap)
                }
                _ => (x, 0.0),
            };
            let mut ranges = Vec::new();
            let mut row_end = row_y;
            for ((index, child_index), item_width) in line.iter().enumerate().zip(sizes.drain(..)) {
                self.y = row_y;
                self.frame = Some((cursor, item_width));
                let start = self.pages.last().map_or(0, |page| page.items.len());
                let mut child = children[*child_index].clone();
                child.style.margin = [0.0; 4];
                child.style.margin_percent = [None; 4];
                child.style.width = Some(item_width);
                child.style.width_percent = None;
                child.style.box_sizing = "border-box".into();
                self.node(&child)?;
                if self.pages.len() != row_page {
                    return Err(Error("flex line cannot split across pages".into()));
                }
                let end = self.pages.last().unwrap().items.len();
                ranges.push((start, end, self.y, child.style.align_self.clone()));
                row_end = row_end.max(self.y);
                cursor += item_width;
                if index + 1 < line.len() {
                    cursor += node.style.column_gap + extra_gap;
                }
            }
            for (start, end, child_end, align_self) in ranges {
                let align = if align_self == "auto" {
                    &node.style.align_items
                } else {
                    &align_self
                };
                let shift = match align.as_str() {
                    "end" => row_end - child_end,
                    "center" => (row_end - child_end) / 2.0,
                    _ => 0.0,
                };
                if align == "stretch" {
                    let height = row_end - row_y;
                    for item in &mut self.pages.last_mut().unwrap().items[start..end] {
                        stretch_box_item(item, row_y, height);
                    }
                } else if shift > EPS {
                    for item in &mut self.pages.last_mut().unwrap().items[start..end] {
                        shift_item(item, shift);
                    }
                }
            }
            let row_end_index = self.pages.last().unwrap().items.len();
            line_records.push((row_start, row_end_index, row_y, row_end - row_y));
            self.y = row_end;
            if line_number + 1 < lines.len() {
                self.y += node.style.row_gap;
            }
        }
        if let Some(height) = declared_height.filter(|_| self.pages.len() == initial_page_count) {
            let used = self.y - start_y;
            let extra = (height - used).max(0.0);
            if extra > EPS {
                let count = line_records.len();
                let (first, between) = match node.style.align_content.as_str() {
                    "end" => (extra, 0.0),
                    "center" => (extra / 2.0, 0.0),
                    "space-between" if count > 1 => (0.0, extra / (count - 1) as f32),
                    "space-around" => {
                        let gap = extra / count as f32;
                        (gap / 2.0, gap)
                    }
                    "stretch" => (0.0, extra / count as f32),
                    _ => (0.0, 0.0),
                };
                for (line_index, (start, end, _, _)) in line_records.iter().enumerate() {
                    let shift = first + between * line_index as f32;
                    for item in &mut self.pages.last_mut().unwrap().items[*start..*end] {
                        shift_item(item, shift);
                    }
                }
                self.y = start_y + height;
            }
        }
        Ok(())
    }

    fn flex_column(
        &mut self,
        node: &Node,
        children: &[&Node],
        x: f32,
        width: f32,
        start_y: f32,
        declared_height: Option<f32>,
    ) -> Result<()> {
        let bases: Vec<Option<f32>> = children
            .iter()
            .map(|child| {
                resolved_dimension(
                    child.style.flex_basis,
                    child.style.flex_basis_percent,
                    declared_height.unwrap_or(self.full_height()),
                )
                .or_else(|| {
                    resolved_dimension(
                        child.style.height,
                        child.style.height_percent,
                        declared_height.unwrap_or(self.full_height()),
                    )
                })
            })
            .collect();
        let total_basis: f32 = bases.iter().flatten().sum();
        let gaps = node.style.row_gap * children.len().saturating_sub(1) as f32;
        let free = declared_height
            .map(|height| height - total_basis - gaps)
            .unwrap_or(0.0);
        let grow_total: f32 = children.iter().map(|child| child.style.flex_grow).sum();
        let shrink_total: f32 = children
            .iter()
            .enumerate()
            .map(|(index, child)| child.style.flex_shrink * bases[index].unwrap_or(0.0))
            .sum();
        let leading = if free > 0.0 && grow_total <= EPS {
            match node.style.justify_content.as_str() {
                "end" => free,
                "center" => free / 2.0,
                "space-around" if !children.is_empty() => free / children.len() as f32 / 2.0,
                _ => 0.0,
            }
        } else {
            0.0
        };
        let extra_gap = if free > 0.0 && grow_total <= EPS {
            match node.style.justify_content.as_str() {
                "space-between" if children.len() > 1 => free / (children.len() - 1) as f32,
                "space-around" if !children.is_empty() => free / children.len() as f32,
                _ => 0.0,
            }
        } else {
            0.0
        };
        self.y = start_y + leading;
        for (index, child) in children.iter().enumerate() {
            let mut allocated = bases[index];
            if let Some(value) = allocated.as_mut() {
                if free > 0.0 && grow_total > EPS {
                    *value += free * child.style.flex_grow / grow_total;
                } else if free < 0.0 && shrink_total > EPS {
                    *value += free * child.style.flex_shrink * *value / shrink_total;
                }
                *value = value.max(EPS);
            }
            if let Some(height) = allocated {
                if self.y + height > self.limit() + EPS && self.page_has_content() {
                    self.new_page()?;
                }
            }
            self.frame = Some((x, width));
            let page = self.pages.len();
            let start = self.pages.last().unwrap().items.len();
            let mut clone = (*child).clone();
            clone.style.margin = [0.0; 4];
            clone.style.margin_percent = [None; 4];
            if let Some(height) = allocated {
                clone.style.height = Some(height);
                clone.style.height_percent = None;
                clone.style.box_sizing = "border-box".into();
            }
            self.node(&clone)?;
            if self.pages.len() != page && allocated.is_some() {
                return Err(Error("sized flex item cannot split across pages".into()));
            }
            if self.pages.len() == page {
                let end = self.pages.last().unwrap().items.len();
                let align = if child.style.align_self == "auto" {
                    node.style.align_items.as_str()
                } else {
                    child.style.align_self.as_str()
                };
                if matches!(align, "center" | "end") {
                    if let Some((left, _, right, _)) =
                        range_bounds(&self.pages.last().unwrap().items[start..end])
                    {
                        let item_width = right - left;
                        let shift = if align == "center" {
                            (width - item_width).max(0.0) / 2.0
                        } else {
                            (width - item_width).max(0.0)
                        };
                        for item in &mut self.pages.last_mut().unwrap().items[start..end] {
                            translate_item(item, shift, 0.0);
                        }
                    }
                }
            }
            if index + 1 < children.len() {
                self.y += node.style.row_gap + extra_gap;
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn flex(&mut self, node: &Node) -> Result<()> {
        if node.style.flex_direction == "row" {
            return self.grid_with_columns(node, node.children.len().max(1));
        }
        self.begin(&node.style)?;
        let old_frame = self.frame;
        let old_center = self.center_children;
        let reference = self.content_width();
        let margins = resolved_edges(node.style.margin, node.style.margin_percent, reference);
        let padding = resolved_edges(node.style.padding, node.style.padding_percent, reference);
        let edges =
            padding[1] + padding[3] + node.style.border_widths[1] + node.style.border_widths[3];
        let available = reference - margins[1] - margins[3];
        let sizing = constrained_dimension(
            resolved_dimension(node.style.width, node.style.width_percent, reference).unwrap_or(
                if node.style.box_sizing == "border-box" {
                    available
                } else {
                    available - edges
                },
            ),
            resolved_dimension(
                node.style.min_width,
                node.style.min_width_percent,
                reference,
            ),
            resolved_dimension(
                node.style.max_width,
                node.style.max_width_percent,
                reference,
            ),
        );
        let outer = if node.style.box_sizing == "border-box" {
            sizing
        } else {
            sizing + edges
        };
        let width = outer - edges;
        let x = self.content_x() + margins[3] + node.style.border_widths[3] + padding[3];
        if width <= 0.0 {
            return Err(Error("flex container has no usable width".into()));
        }
        self.y += margins[0] + node.style.border_widths[0] + padding[0];
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
        let height = constrained_dimension(
            resolved_dimension(
                node.style.height,
                node.style.height_percent,
                self.full_height(),
            )
            .unwrap_or(self.y - start_y),
            resolved_dimension(
                node.style.min_height,
                node.style.min_height_percent,
                self.full_height(),
            ),
            resolved_dimension(
                node.style.max_height,
                node.style.max_height_percent,
                self.full_height(),
            ),
        );
        let constrained_height = node.style.height.is_some()
            || node.style.height_percent.is_some()
            || node.style.min_height.is_some()
            || node.style.min_height_percent.is_some()
            || node.style.max_height.is_some()
            || node.style.max_height_percent.is_some();
        if start_page != self.pages.len() {
            if constrained_height {
                return Err(Error(
                    "height-constrained flex container cannot split across pages".into(),
                ));
            }
        } else {
            let used = self.y - start_y;
            if height > used && node.style.justify_content == "center" {
                let shift = (height - used) / 2.0;
                for item in &mut self.pages.last_mut().expect("page").items[start_item..] {
                    shift_item(item, shift);
                }
            }
            self.y = start_y + height.max(used);
        }
        self.y += padding[2] + node.style.border_widths[2] + margins[2];
        self.finish(&node.style);
        Ok(())
    }
    fn grid_advanced(&mut self, node: &Node) -> Result<()> {
        self.begin(&node.style)?;
        let ContainerGeometry {
            margins,
            padding,
            outer: _outer,
            width,
            x,
        } = self.container_geometry(&node.style, "grid")?;
        self.y += margins[0] + node.style.border_widths[0] + padding[0];
        let start_y = self.y;
        let start_pages = self.pages.len();
        let old_frame = self.frame;
        let declared_height = resolved_dimension(
            node.style.height,
            node.style.height_percent,
            self.full_height(),
        );
        let establishes = node.style.position != "static";
        if establishes {
            self.containing_blocks.push(ContainingBlock {
                x,
                y: start_y,
                w: width,
                h: declared_height.unwrap_or_else(|| (self.limit() - start_y).max(EPS)),
                clip: node.style.overflow == "hidden",
            });
        }
        for child in node.children.iter().filter(|child| {
            child.tag != "#text" && matches!(child.style.position.as_str(), "absolute" | "fixed")
        }) {
            self.node(child)?;
        }
        let children: Vec<_> = node
            .children
            .iter()
            .filter(|child| {
                (child.tag == "#text"
                    || !matches!(child.style.position.as_str(), "absolute" | "fixed"))
                    && (child.tag != "#text" || !child.text.trim().is_empty())
            })
            .collect();
        let tracks = if node.style.grid_template_columns.is_empty() {
            vec![css::GridTrack::Fr(1.0); node.style.grid_columns.unwrap_or(1).max(1)]
        } else {
            node.style.grid_template_columns.clone()
        };
        let column_widths = resolve_grid_tracks(&tracks, width, node.style.column_gap)?;
        let columns = column_widths.len();
        let tracks_width = column_widths.iter().sum::<f32>()
            + node.style.column_gap * columns.saturating_sub(1) as f32;
        let horizontal_extra = (width - tracks_width).max(0.0);
        let (grid_x, grid_column_gap) = match node.style.justify_content.as_str() {
            "end" => (x + horizontal_extra, node.style.column_gap),
            "center" => (x + horizontal_extra / 2.0, node.style.column_gap),
            "space-between" if columns > 1 => (
                x,
                node.style.column_gap + horizontal_extra / (columns - 1) as f32,
            ),
            "space-around" => {
                let extra_gap = horizontal_extra / columns as f32;
                (x + extra_gap / 2.0, node.style.column_gap + extra_gap)
            }
            _ => (x, node.style.column_gap),
        };
        let mut occupancy: Vec<Vec<bool>> = Vec::new();
        let mut placements: Vec<(&Node, usize, usize, usize, usize)> = Vec::new();
        let ensure_rows = |occupancy: &mut Vec<Vec<bool>>, rows: usize| {
            while occupancy.len() < rows {
                occupancy.push(vec![false; columns]);
            }
        };
        for child in children {
            let column_span = child.style.grid_column_span.min(columns).max(1);
            let row_span = child.style.grid_row_span.max(1);
            let explicit_column = child.style.grid_column_start.map(|value| value - 1);
            let explicit_row = child.style.grid_row_start.map(|value| value - 1);
            let (row, column) = if let (Some(row), Some(column)) = (explicit_row, explicit_column) {
                if column + column_span > columns {
                    return Err(Error("grid placement exceeds explicit columns".into()));
                }
                ensure_rows(&mut occupancy, row + row_span);
                (row, column)
            } else {
                let mut row = explicit_row.unwrap_or(0);
                let mut found = None;
                while found.is_none() {
                    ensure_rows(&mut occupancy, row + row_span);
                    let columns_to_try: Box<dyn Iterator<Item = usize>> =
                        if let Some(column) = explicit_column {
                            Box::new(std::iter::once(column))
                        } else {
                            Box::new(0..=columns - column_span)
                        };
                    for column in columns_to_try {
                        if column + column_span <= columns
                            && (row..row + row_span).all(|grid_row| {
                                occupancy[grid_row][column..column + column_span]
                                    .iter()
                                    .all(|occupied| !occupied)
                            })
                        {
                            found = Some((row, column));
                            break;
                        }
                    }
                    if explicit_row.is_some() && found.is_none() {
                        return Err(Error(
                            "explicit grid placement overlaps another item".into(),
                        ));
                    }
                    row += 1;
                }
                found.unwrap()
            };
            if (row..row + row_span).any(|grid_row| {
                occupancy[grid_row][column..column + column_span]
                    .iter()
                    .any(|occupied| *occupied)
            }) {
                return Err(Error(
                    "explicit grid placement overlaps another item".into(),
                ));
            }
            for grid_row in &mut occupancy[row..row + row_span] {
                grid_row[column..column + column_span].fill(true);
            }
            placements.push((child, row, column, row_span, column_span));
        }
        let row_count = occupancy
            .len()
            .max(node.style.grid_template_rows.len())
            .max(1);
        let mut row_sizes = vec![None; row_count];
        if !node.style.grid_template_rows.is_empty() {
            if let Some(height) = declared_height {
                let resolved = resolve_grid_tracks(
                    &node.style.grid_template_rows,
                    height,
                    node.style.row_gap,
                )?;
                for (slot, value) in row_sizes.iter_mut().zip(resolved) {
                    *slot = Some(value);
                }
            } else {
                for (slot, track) in row_sizes.iter_mut().zip(&node.style.grid_template_rows) {
                    *slot = match track {
                        css::GridTrack::Fixed(value) => Some(*value),
                        css::GridTrack::Percent(value) => Some(self.full_height() * value),
                        css::GridTrack::MinMax(minimum, _) => match minimum.as_ref() {
                            css::GridTrack::Fixed(value) => Some(*value),
                            css::GridTrack::Percent(value) => Some(self.full_height() * value),
                            _ => None,
                        },
                        _ => None,
                    };
                }
            }
        }
        self.y = start_y;
        let initial_page_count = self.pages.len();
        let mut row_records = Vec::new();
        for (row, row_size) in row_sizes.iter().copied().enumerate().take(row_count) {
            let estimate = row_size.unwrap_or(40.0);
            if self.y + estimate > self.limit() + EPS && self.page_has_content() {
                self.new_page()?;
            }
            let row_page = self.pages.len();
            let row_y = self.y;
            let row_item_start = self.pages.last().map_or(0, |page| page.items.len());
            let mut row_end = row_y + row_size.unwrap_or(0.0);
            let mut ranges = Vec::new();
            for (child, _, column, row_span, column_span) in
                placements.iter().filter(|placement| placement.1 == row)
            {
                let cell_x = grid_x
                    + column_widths[..*column].iter().sum::<f32>()
                    + grid_column_gap * *column as f32;
                let cell_width = column_widths[*column..*column + *column_span]
                    .iter()
                    .sum::<f32>()
                    + grid_column_gap * column_span.saturating_sub(1) as f32;
                let justify = if child.style.justify_self == "auto" {
                    node.style.justify_items.as_str()
                } else {
                    child.style.justify_self.as_str()
                };
                let requested_width =
                    resolved_dimension(child.style.width, child.style.width_percent, cell_width);
                let item_width = if justify == "stretch" {
                    cell_width
                } else {
                    requested_width.unwrap_or(cell_width).min(cell_width)
                };
                let offset = match justify {
                    "end" => cell_width - item_width,
                    "center" => (cell_width - item_width) / 2.0,
                    _ => 0.0,
                };
                self.y = row_y;
                self.frame = Some((cell_x + offset.max(0.0), item_width));
                let start = self.pages.last().unwrap().items.len();
                let mut clone = (*child).clone();
                clone.style.margin = [0.0; 4];
                clone.style.margin_percent = [None; 4];
                if justify == "stretch" || requested_width.is_none() {
                    clone.style.width = Some(item_width);
                    clone.style.width_percent = None;
                    clone.style.box_sizing = "border-box".into();
                }
                let align = if clone.style.align_self == "auto" {
                    node.style.align_items.clone()
                } else {
                    clone.style.align_self.clone()
                };
                if *row_span == 1 && align == "stretch" {
                    if let Some(height) = row_size {
                        clone.style.height = Some(height);
                        clone.style.height_percent = None;
                        clone.style.box_sizing = "border-box".into();
                    }
                }
                self.node(&clone)?;
                if self.pages.len() != row_page {
                    return Err(Error("grid row cannot split across pages".into()));
                }
                let end = self.pages.last().unwrap().items.len();
                let child_end = self.y;
                let contribution = (child_end - row_y) / *row_span as f32;
                row_end = row_end.max(row_y + contribution.max(0.0));
                ranges.push((start, end, child_end, align));
            }
            if row_end <= row_y + EPS {
                row_end = row_y + row_size.unwrap_or(0.0);
            }
            for (start, end, child_end, align) in ranges {
                let shift = match align.as_str() {
                    "end" => row_end - child_end,
                    "center" => (row_end - child_end) / 2.0,
                    _ => 0.0,
                };
                if align == "stretch" {
                    let height = row_end - row_y;
                    for item in &mut self.pages.last_mut().unwrap().items[start..end] {
                        stretch_box_item(item, row_y, height);
                    }
                } else if shift > EPS {
                    for item in &mut self.pages.last_mut().unwrap().items[start..end] {
                        shift_item(item, shift);
                    }
                }
            }
            row_records.push((
                row_page - 1,
                row_item_start,
                self.pages.last().map_or(0, |page| page.items.len()),
            ));
            self.y = row_end;
            if row + 1 < row_count {
                self.y += node.style.row_gap;
            }
        }
        if let Some(height) = declared_height.filter(|_| self.pages.len() == initial_page_count) {
            let extra = (height - (self.y - start_y)).max(0.0);
            if extra > EPS {
                let (first, between) = match node.style.align_content.as_str() {
                    "end" => (extra, 0.0),
                    "center" => (extra / 2.0, 0.0),
                    "space-between" if row_count > 1 => (0.0, extra / (row_count - 1) as f32),
                    "space-around" => {
                        let gap = extra / row_count as f32;
                        (gap / 2.0, gap)
                    }
                    "stretch" => (0.0, extra / row_count as f32),
                    _ => (0.0, 0.0),
                };
                for (row, (page, start, end)) in row_records.iter().enumerate() {
                    let shift = first + between * row as f32;
                    for item in &mut self.pages[*page].items[*start..*end] {
                        shift_item(item, shift);
                    }
                }
                self.y = start_y + height;
            }
        }
        if establishes {
            self.containing_blocks.pop();
        }
        self.frame = old_frame;
        let used = if self.pages.len() == start_pages {
            self.y - start_y
        } else {
            0.0
        };
        let height = constrained_dimension(
            declared_height.unwrap_or(used),
            resolved_dimension(
                node.style.min_height,
                node.style.min_height_percent,
                self.full_height(),
            ),
            resolved_dimension(
                node.style.max_height,
                node.style.max_height_percent,
                self.full_height(),
            ),
        );
        let constrained = declared_height.is_some()
            || node.style.min_height.is_some()
            || node.style.min_height_percent.is_some()
            || node.style.max_height.is_some()
            || node.style.max_height_percent.is_some();
        if self.pages.len() != start_pages && constrained {
            return Err(Error(
                "height-constrained grid container cannot split across pages".into(),
            ));
        }
        if self.pages.len() == start_pages {
            self.y = start_y + used.max(height);
        }
        self.y += padding[2] + node.style.border_widths[2] + margins[2];
        self.finish(&node.style);
        Ok(())
    }

    #[allow(dead_code)]
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
        let margins = resolved_edges(node.style.margin, node.style.margin_percent, outer_width);
        let padding = resolved_edges(node.style.padding, node.style.padding_percent, outer_width);
        let edges =
            padding[1] + padding[3] + node.style.border_widths[1] + node.style.border_widths[3];
        let available = outer_width - margins[1] - margins[3];
        let sizing = constrained_dimension(
            resolved_dimension(node.style.width, node.style.width_percent, outer_width).unwrap_or(
                if node.style.box_sizing == "border-box" {
                    available
                } else {
                    available - edges
                },
            ),
            resolved_dimension(
                node.style.min_width,
                node.style.min_width_percent,
                outer_width,
            ),
            resolved_dimension(
                node.style.max_width,
                node.style.max_width_percent,
                outer_width,
            ),
        );
        let outer = if node.style.box_sizing == "border-box" {
            sizing
        } else {
            sizing + edges
        };
        let width = outer - edges;
        let x = outer_x + margins[3] + node.style.border_widths[3] + padding[3];
        if width <= 0.0 {
            return Err(Error("grid container has no usable width".into()));
        }
        self.y += margins[0] + node.style.border_widths[0] + padding[0];
        let start_y = self.y;
        let start_page = self.pages.len();
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
        let used = self.y - start_y;
        let constrained_height = node.style.height.is_some()
            || node.style.height_percent.is_some()
            || node.style.min_height.is_some()
            || node.style.min_height_percent.is_some()
            || node.style.max_height.is_some()
            || node.style.max_height_percent.is_some();
        if start_page != self.pages.len() && constrained_height {
            return Err(Error(
                "height-constrained grid container cannot split across pages".into(),
            ));
        }
        let height = constrained_dimension(
            resolved_dimension(
                node.style.height,
                node.style.height_percent,
                self.full_height(),
            )
            .unwrap_or(used),
            resolved_dimension(
                node.style.min_height,
                node.style.min_height_percent,
                self.full_height(),
            ),
            resolved_dimension(
                node.style.max_height,
                node.style.max_height_percent,
                self.full_height(),
            ),
        );
        if start_page == self.pages.len() {
            self.y = start_y
                + if node.style.overflow == "hidden" {
                    height
                } else {
                    used.max(height)
                };
        }
        self.y += padding[2] + node.style.border_widths[2] + margins[2];
        self.finish(&node.style);
        Ok(())
    }
    fn paint_box(&mut self, x: f32, y: f32, w: f32, h: f32, style: &Style) {
        let uniform_border = style
            .border_widths
            .iter()
            .all(|value| (*value - style.border_widths[0]).abs() <= EPS)
            && style
                .border_colors
                .iter()
                .all(|value| *value == style.border_colors[0])
            && style
                .border_alphas
                .iter()
                .all(|value| (*value - 1.0).abs() <= EPS)
            && style.border_styles.iter().all(|value| {
                value == "solid" || (style.border_widths[0] <= EPS && value == "none")
            });
        let legacy = uniform_border
            && style.border_radius.iter().all(|value| *value <= EPS)
            && style.opacity >= 1.0 - EPS
            && style.background_alpha >= 1.0 - EPS;
        if style.visibility == "visible"
            && legacy
            && (style.background.is_some() || style.border_widths[0] > EPS)
        {
            self.item(Item::Rect {
                x,
                y,
                w,
                h,
                fill: style.background,
                stroke: (style.border_widths[0] > EPS)
                    .then_some((style.border_widths[0], style.border_colors[0])),
            });
            return;
        }
        if let ("visible", Some(background)) = (style.visibility.as_str(), style.background) {
            self.item(painted_rect(
                x,
                y,
                w,
                h,
                background,
                style.background_alpha * style.opacity,
                style.border_radius,
            ));
        }
        if style.visibility == "visible" && style.border_widths.iter().any(|value| *value > 0.0) {
            self.item(Item::Border {
                x,
                y,
                w,
                h,
                widths: style.border_widths,
                colors: style.border_colors,
                alphas: style.border_alphas.map(|alpha| alpha * style.opacity),
                styles: style.border_styles.clone(),
                radius: style.border_radius,
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
            let item_start = self.pages.last().map_or(0, |page| page.items.len());
            let baseline = y + line.baseline - glyph.shift;
            if let Some(name) = &glyph.destination {
                self.visible_item(Item::Destination {
                    name: name.clone(),
                    x: pen,
                    y,
                });
            }
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
                self.paint_box(box_x, box_y, inline.width, inline.height, &inline.style);
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
                if glyph.paint_late && line.actual_text.is_none() {
                    self.defer_inline_paint(item_start);
                }
                continue;
            }
            if glyph.visible && (!glyph.soft_hyphen || glyph.advance > 0.0) {
                if glyph.alpha < 1.0 - EPS {
                    self.visible_item(Item::BeginOpacity(glyph.alpha));
                }
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
                if glyph.alpha < 1.0 - EPS {
                    self.visible_item(Item::EndOpacity);
                }
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
                if glyph.alpha < 1.0 - EPS {
                    self.visible_item(Item::BeginOpacity(glyph.alpha));
                }
                self.visible_item(Item::Rect {
                    x: pen,
                    y: decoration_y,
                    w: glyph.advance.max(0.0),
                    h: (glyph.size * 0.055).max(0.35),
                    fill: Some(glyph.color),
                    stroke: None,
                });
                if glyph.alpha < 1.0 - EPS {
                    self.visible_item(Item::EndOpacity);
                }
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
            if glyph.paint_late && line.actual_text.is_none() {
                self.defer_inline_paint(item_start);
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
        let margins = resolved_edges(s.margin, s.margin_percent, containing_width);
        let padding = resolved_edges(s.padding, s.padding_percent, containing_width);
        let borders = s.border_widths;
        let x = self.content_x() + margins[3];
        let horizontal_edges = padding[1] + padding[3] + borders[1] + borders[3];
        let available_outer = containing_width - margins[1] - margins[3];
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
        let vertical_edges = padding[0] + padding[2] + borders[0] + borders[2];
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
        let box_height = if s.overflow == "hidden" {
            requested_outer_height
        } else {
            natural_height.max(requested_outer_height)
        };
        if (declared_height.is_some() || minimum_height.is_some())
            && requested_outer_height > self.full_height() + EPS
        {
            return Err(Error("paragraph box exceeds page content height".into()));
        }
        let full_needed = margins[0] + box_height + margins[2];
        if (s.break_inside_avoid || lines.len() == 1)
            && full_needed <= self.full_height() + EPS
            && self.y + full_needed > self.limit() + EPS
            && self.page_has_content()
        {
            self.new_page()?;
        }
        self.y += margins[0];
        let first_box_y = self.y;
        let mut offset = 0;
        while offset < lines.len() {
            let start = self.y;
            let overhead = padding[0] + padding[2] + borders[0] + borders[2];
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
                if matches!(node.tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                    let title = node
                        .plain_text()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !title.is_empty() {
                        let destination = node.attr("id").map(str::to_owned).unwrap_or_else(|| {
                            let name = format!("_serpentype_outline_{}", self.outlines.len());
                            self.item(Item::Destination {
                                name: name.clone(),
                                x,
                                y: start,
                            });
                            name
                        });
                        let level = node
                            .tag
                            .strip_prefix('h')
                            .and_then(|number| number.parse::<u8>().ok())
                            .unwrap_or(1);
                        self.outlines.push(OutlineEntry {
                            title,
                            destination,
                            level,
                        });
                    }
                }
            }
            self.paint_box(x, start, w, used, s);
            if s.overflow == "hidden" {
                self.visible_item(Item::BeginClip {
                    x,
                    y: start,
                    w,
                    h: box_height,
                    radius: s.border_radius,
                });
            }
            let mut line_y = start + borders[0] + padding[0];
            for (line_index, line) in lines[offset..end].iter().enumerate() {
                let absolute_index = offset + line_index;
                let first_indent = if absolute_index == 0 {
                    s.text_indent
                } else {
                    0.0
                };
                self.draw_line(
                    line,
                    x + borders[3] + padding[3] + first_indent,
                    line_y,
                    inner - first_indent,
                    &s.text_align,
                    absolute_index + 1 == lines.len(),
                );
                line_y += line.height;
            }
            if s.overflow == "hidden" {
                self.visible_item(Item::EndClip);
            }
            self.y += used;
            offset = end;
            if offset < lines.len() {
                self.new_page()?;
            }
        }
        if s.overflow == "hidden" {
            self.y = first_box_y + box_height;
        }
        self.y += margins[2];
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
            for font_digest in self.fonts.source_digests()? {
                hash.update(font_digest);
            }
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
                        svg: None,
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
                        svg: None,
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
        let mut box_width = node
            .style
            .width
            .or_else(|| node.style.width_percent.map(|p| self.content_width() * p))
            .unwrap_or(iw)
            .min(self.content_width());
        box_width = constrained_dimension(
            box_width,
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
        let mut box_height = node
            .style
            .height
            .or_else(|| node.style.height_percent.map(|p| self.full_height() * p))
            .unwrap_or_else(|| box_width / node.style.aspect_ratio.unwrap_or(iw / ih));
        if let Some(max_height) = resolved_dimension(
            node.style.max_height,
            node.style.max_height_percent,
            self.full_height(),
        ) {
            if box_height > max_height {
                let scale = max_height / box_height;
                box_height = max_height;
                box_width *= scale;
            }
        }
        if let Some(min_height) = resolved_dimension(
            node.style.min_height,
            node.style.min_height_percent,
            self.full_height(),
        ) {
            if box_height < min_height {
                let scale = min_height / box_height;
                box_height = min_height;
                box_width *= scale;
            }
        }
        if box_height > self.full_height() + EPS {
            return Err(Error("image exceeds page content height".into()));
        }
        if self.y + box_height > self.limit() + EPS && self.page_has_content() {
            self.new_page()?;
        }
        let available_width = self.content_width() - box_width;
        let inline_alignment = match node.style.text_align.as_str() {
            "center" => available_width.max(0.0) / 2.0,
            "right" => available_width.max(0.0),
            _ => 0.0,
        };
        let box_x = self.content_x()
            + if self.center_children {
                available_width.max(0.0) / 2.0
            } else {
                inline_alignment
            };
        let (paint_width, paint_height) = match node.style.object_fit.as_str() {
            "contain" => {
                let scale = (box_width / iw).min(box_height / ih);
                (iw * scale, ih * scale)
            }
            "cover" => {
                let scale = (box_width / iw).max(box_height / ih);
                (iw * scale, ih * scale)
            }
            "none" => (iw, ih),
            "scale-down" => {
                let scale = 1.0_f32.min((box_width / iw).min(box_height / ih));
                (iw * scale, ih * scale)
            }
            _ => (box_width, box_height),
        };
        let paint_x = box_x
            + (box_width - paint_width) * node.style.object_position[0]
            + node.style.object_position_offset[0];
        let paint_y = self.y
            + (box_height - paint_height) * node.style.object_position[1]
            + node.style.object_position_offset[1];
        let data = if let Some(max_dpi) = self.max_image_dpi.filter(|_| data.svg.is_none()) {
            match downsample_image(&data, max_dpi, paint_width, paint_height) {
                Ok(downsampled) => Arc::new(downsampled),
                Err(error) if error.0 == "image does not require downsampling" => data,
                Err(error) => return Err(error),
            }
        } else {
            data
        };
        self.destination(node, box_x, self.y);
        if node.style.opacity < 1.0 - EPS {
            self.item(Item::BeginOpacity(node.style.opacity));
        }
        if node.style.object_fit != "fill" {
            self.item(Item::BeginClip {
                x: box_x,
                y: self.y,
                w: box_width,
                h: box_height,
                radius: [0.0; 4],
            });
        }
        self.item(Item::Image {
            data,
            x: paint_x,
            y: paint_y,
            w: paint_width,
            h: paint_height,
        });
        if node.style.object_fit != "fill" {
            self.item(Item::EndClip);
        }
        if node.style.opacity < 1.0 - EPS {
            self.item(Item::EndOpacity);
        }
        self.y += box_height;
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
    bottom_extra: f32,
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
        let spacing = if table.style.border_collapse == "separate" {
            table.style.border_spacing
        } else {
            [0.0; 2]
        };
        let (collapsed_left_inset, collapsed_right_inset) =
            if table.style.border_collapse == "collapse" {
                let left = row_nodes
                    .iter()
                    .flat_map(|row| &row.cells)
                    .filter(|cell| cell.column == 0)
                    .map(|cell| cell.node.style.border_widths[3])
                    .fold(0.0f32, f32::max)
                    / 2.0;
                let right = row_nodes
                    .iter()
                    .flat_map(|row| &row.cells)
                    .filter(|cell| cell.column + cell.colspan == columns)
                    .map(|cell| cell.node.style.border_widths[1])
                    .fold(0.0f32, f32::max)
                    / 2.0;
                (left, right)
            } else {
                (0.0, 0.0)
            };
        let table_x = self.content_x() + table.style.margin[3] + collapsed_left_inset;
        let track_width = width
            - spacing[0] * columns.saturating_sub(1) as f32
            - collapsed_left_inset
            - collapsed_right_inset;
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
            if let Some(last_row) = foots.last_mut().or_else(|| bodies.last_mut()) {
                last_row.bottom_extra = last_row
                    .cells
                    .iter()
                    .map(|cell| cell.borders[2].width)
                    .fold(0.0f32, f32::max);
            }
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
                let bottom_extra = if offset + remaining >= row.line_count {
                    row.bottom_extra
                } else {
                    0.0
                };
                let row_height = remaining as f32 * row.step + row.pad + row.row_gap + bottom_extra;
                if row_height <= self.full_height() - header_height - footer_height + EPS
                    && self.y + row_height > self.limit() - footer_height + EPS
                    && self.page_has_content()
                {
                    if let Some(previous) =
                        row_index.checked_sub(1).and_then(|index| bodies.get(index))
                    {
                        self.paint_collapsed_row_boundary(row, &widths, self.y - previous.row_gap);
                    }
                    self.draw_table_footer(&foots, &widths);
                    self.new_page()?;
                    self.ensure_header(&heads, &widths, header_height, footer_height, row)?;
                }
                let available =
                    self.limit() - footer_height - self.y - row.pad - row.row_gap - bottom_extra;
                let take = ((available + EPS) / row.step).floor().max(0.0) as usize;
                if take == 0 {
                    if self.y > self.page.margin[0] + header_height + EPS {
                        if let Some(previous) =
                            row_index.checked_sub(1).and_then(|index| bodies.get(index))
                        {
                            self.paint_collapsed_row_boundary(
                                row,
                                &widths,
                                self.y - previous.row_gap,
                            );
                        }
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
                    self.paint_collapsed_row_boundary(row, &widths, self.y - row.row_gap);
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
            self.source.clone(),
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
    fn cell_border_inset(border_width: f32, collapsed: bool) -> f32 {
        if collapsed {
            border_width / 2.0
        } else {
            border_width
        }
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
            let border_inset = Self::cell_border_inset(s.border_width, collapsed);
            let inner = width - s.padding[1] - s.padding[3] - 2.0 * border_inset;
            if inner <= 0.0 {
                return Err(Error("table cell has no usable width".into()));
            }
            let (lines, items, content_height) = if Self::contains_nested_table(cell) {
                let (items, height) = self.layout_nested_cell(cell, inner)?;
                (vec![], items, height)
            } else {
                let mut line_node = cell.clone();
                fn disable_font_bbox(node: &mut Node) {
                    node.style.use_font_bbox_for_line_height = false;
                    for child in &mut node.children {
                        disable_font_bbox(child);
                    }
                }
                disable_font_bbox(&mut line_node);
                let lines = lines_for(
                    &line_node,
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
                let edges = s.padding[0] + s.padding[2] + 2.0 * border_inset;
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
            bottom_extra: 0.0,
            x,
            column_gap: spacing[0],
            row_gap: spacing[1],
            collapsed,
        })
    }
    fn table_row_height(row: &Row) -> f32 {
        row.line_count as f32 * row.step + row.pad + row.row_gap + row.bottom_extra
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
            self.item(painted_rect(
                x,
                y,
                width,
                height,
                fill,
                cell.style.background_alpha * cell.style.opacity,
                cell.style.border_radius,
            ));
        }
        let mut edge = |side: usize, ex: f32, ey: f32, ew: f32, eh: f32| {
            let border = cell.borders[side];
            if cell.draw_borders[side] && border.width > 0.0 {
                if cell.style.opacity < 1.0 - EPS {
                    self.item(Item::BeginOpacity(cell.style.opacity));
                }
                self.item(Item::Rect {
                    x: ex,
                    y: ey,
                    w: ew.max(border.width),
                    h: eh.max(border.width),
                    fill: Some(border.color),
                    stroke: None,
                });
                if cell.style.opacity < 1.0 - EPS {
                    self.item(Item::EndOpacity);
                }
            }
        };
        if first_fragment {
            edge(0, x, y, width, cell.borders[0].width);
        }
        edge(
            1,
            x + width - cell.borders[1].width / 2.0,
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
        edge(
            3,
            x - cell.borders[3].width / 2.0,
            y,
            cell.borders[3].width,
            height,
        );
    }
    fn draw_cell_items(&mut self, cell: &Cell, x: f32, y: f32) {
        for original in &cell.items {
            let mut item = original.clone();
            translate_item(&mut item, x, y);
            self.item(item);
        }
    }
    fn paint_collapsed_row_boundary(&mut self, row: &Row, widths: &[f32], y: f32) {
        if !row.collapsed {
            return;
        }
        for cell in &row.cells {
            let border = cell.borders[0];
            if !cell.draw_borders[0] || border.width <= 0.0 || cell.style.visibility != "visible" {
                continue;
            }
            let x = row.x
                + widths[..cell.column].iter().sum::<f32>()
                + row.column_gap * cell.column as f32;
            let width: f32 = widths[cell.column..cell.column + cell.colspan]
                .iter()
                .sum::<f32>()
                + row.column_gap * cell.colspan.saturating_sub(1) as f32;
            if cell.style.opacity < 1.0 - EPS {
                self.item(Item::BeginOpacity(cell.style.opacity));
            }
            self.item(Item::Rect {
                x,
                y,
                w: width.max(border.width),
                h: border.width,
                fill: Some(border.color),
                stroke: None,
            });
            if cell.style.opacity < 1.0 - EPS {
                self.item(Item::EndOpacity);
            }
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
                let border_inset = Self::cell_border_inset(cell.style.border_width, row.collapsed);
                let natural = cell.content_height
                    + cell.style.padding[0]
                    + cell.style.padding[2]
                    + 2.0 * border_inset;
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
                    let border_inset =
                        Self::cell_border_inset(cell.style.border_width, row.collapsed);
                    let content_top = offsets[local_row]
                        + border_inset
                        + cell.style.padding[0]
                        + Self::table_cell_vertical_offset(
                            &cell.style,
                            cell_height,
                            cell.content_height,
                            row.collapsed,
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
                let border_inset = Self::cell_border_inset(cell.style.border_width, row.collapsed);
                let text_x = x + border_inset + cell.style.padding[3];
                let cell_height = cell_end - cell_start;
                let content_top = cell_start
                    + border_inset
                    + cell.style.padding[0]
                    + Self::table_cell_vertical_offset(
                        &cell.style,
                        cell_height,
                        cell.content_height,
                        row.collapsed,
                    );
                let inner_width =
                    width - cell.style.padding[1] - cell.style.padding[3] - 2.0 * border_inset;
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
    fn table_cell_vertical_offset(
        style: &Style,
        box_height: f32,
        content_height: f32,
        collapsed: bool,
    ) -> f32 {
        let border_inset = Self::cell_border_inset(style.border_width, collapsed);
        let available = (box_height
            - style.padding[0]
            - style.padding[2]
            - 2.0 * border_inset
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
        let bottom_extra = if start + count >= row.line_count {
            row.bottom_extra
        } else {
            0.0
        };
        let y = self.y;
        for cell in &row.cells {
            let x = row.x
                + widths[..cell.column].iter().sum::<f32>()
                + row.column_gap * cell.column as f32;
            let w: f32 = widths[cell.column..cell.column + cell.colspan]
                .iter()
                .sum::<f32>()
                + row.column_gap * cell.colspan.saturating_sub(1) as f32;
            self.paint_table_cell(
                cell,
                (x, y, w, height + bottom_extra),
                (true, true),
                row.collapsed,
            );
            let border_inset = Self::cell_border_inset(cell.style.border_width, row.collapsed);
            let text_x = x + border_inset + cell.style.padding[3];
            let visible_lines = cell.lines.len().saturating_sub(start).min(count);
            let content_height = if cell.items.is_empty() {
                visible_lines as f32 * row.step
            } else {
                cell.content_height
            };
            let vertical_offset = if start == 0 && count >= cell.lines.len() {
                Self::table_cell_vertical_offset(&cell.style, height, content_height, row.collapsed)
            } else {
                0.0
            };
            let mut line_y = y + cell.style.border_width + cell.style.padding[0] + vertical_offset;
            for (local_index, line) in cell.lines.iter().skip(start).take(count).enumerate() {
                self.draw_line(
                    line,
                    text_x,
                    line_y,
                    w - cell.style.padding[1] - cell.style.padding[3] - 2.0 * border_inset,
                    &cell.style.text_align,
                    start + local_index + 1 == cell.lines.len(),
                );
                line_y += row.step;
            }
            if !cell.items.is_empty() && start == 0 {
                self.draw_cell_items(cell, text_x, line_y);
            }
        }
        self.y += height + bottom_extra;
        if start + count >= row.line_count {
            self.y += row.row_gap;
        }
    }
}
