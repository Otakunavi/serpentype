//! HTML tree extraction and style resolution for the supported document tags.
use crate::css::{self, Sheet, Style};
use crate::{Diagnostic, Error, Result};
use kuchiki::traits::TendrilSink;
use kuchiki::NodeRef;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug, Default)]
pub struct DocumentMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub language: Option<String>,
}

/// An owned styled DOM node. It contains no parser references and is safe to share.
#[derive(Clone, Debug)]
pub struct Node {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub style: Style,
    pub text: String,
    pub children: Vec<Node>,
    /// Byte offset of the corresponding opening element in the source HTML, when available.
    pub source_offset: Option<usize>,
}
impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }
    pub fn plain_text(&self) -> String {
        if self.tag == "br" {
            return "\n".into();
        }
        let mut s = self.text.clone();
        for c in &self.children {
            s.push_str(&c.plain_text());
        }
        s
    }
}

const TAGS: &[&str] = &[
    "html", "head", "body", "style", "div", "section", "article", "p", "h1", "h2", "h3", "h4",
    "h5", "h6", "span", "strong", "b", "em", "i", "u", "a", "sup", "sub", "br", "img", "table",
    "colgroup", "col", "thead", "tbody", "tfoot", "tr", "th", "td",
];

#[derive(Default)]
struct SourceOffsets {
    elements: HashMap<String, VecDeque<usize>>,
}

impl SourceOffsets {
    fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        let lower = source.to_ascii_lowercase();
        let mut elements: HashMap<String, VecDeque<usize>> = HashMap::new();
        let mut cursor = 0;
        while cursor < bytes.len() {
            if bytes[cursor] != b'<' {
                cursor += 1;
                continue;
            }
            let start = cursor;
            cursor += 1;
            if source[start..].starts_with("<!--") {
                cursor = source[start + 4..]
                    .find("-->")
                    .map_or(bytes.len(), |end| start + 4 + end + 3);
                continue;
            }
            if cursor >= bytes.len() || matches!(bytes[cursor], b'/' | b'!' | b'?') {
                cursor = bytes[cursor..]
                    .iter()
                    .position(|byte| *byte == b'>')
                    .map_or(bytes.len(), |end| cursor + end + 1);
                continue;
            }
            let name_start = cursor;
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b'-' | b':'))
            {
                cursor += 1;
            }
            if cursor == name_start {
                continue;
            }
            let name = String::from_utf8_lossy(&bytes[name_start..cursor]).to_ascii_lowercase();
            elements.entry(name.clone()).or_default().push_back(start);
            let mut quote = None;
            while cursor < bytes.len() {
                let byte = bytes[cursor];
                if let Some(end_quote) = quote {
                    if byte == end_quote {
                        quote = None;
                    }
                } else if matches!(byte, b'\'' | b'"') {
                    quote = Some(byte);
                } else if byte == b'>' {
                    cursor += 1;
                    break;
                }
                cursor += 1;
            }
            if matches!(
                name.as_str(),
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
                let closing = format!("</{name}");
                cursor = lower[cursor..]
                    .find(&closing)
                    .and_then(|offset| {
                        lower[cursor + offset..]
                            .find('>')
                            .map(|end| cursor + offset + end + 1)
                    })
                    .unwrap_or(bytes.len());
            }
        }
        Self { elements }
    }

    fn take(&mut self, tag: &str) -> Option<usize> {
        self.elements.get_mut(tag).and_then(VecDeque::pop_front)
    }
}

fn defaults(tag: &str, style: &mut Style) {
    if matches!(
        tag,
        "span" | "strong" | "b" | "em" | "i" | "u" | "a" | "sup" | "sub" | "br"
    ) {
        style.display = "inline".into();
    }
    if matches!(tag, "td" | "th") {
        style.display = "table-cell".into();
        style.vertical_align = "middle".into();
    }
    match tag {
        "h1" => {
            style.font_size = 24.0;
            style.line_height = style
                .line_height_factor
                .map_or(style.line_height, |factor| style.font_size * factor);
            style.weight = 700;
            style.margin = [12.0, 0.0, 12.0, 0.0]
        }
        "h2" => {
            style.font_size = 18.0;
            style.line_height = style
                .line_height_factor
                .map_or(style.line_height, |factor| style.font_size * factor);
            style.weight = 700;
            style.margin = [9.0, 0.0, 9.0, 0.0]
        }
        "h3" | "h4" | "h5" | "h6" => {
            style.weight = 700;
            style.margin = [7.0, 0.0, 7.0, 0.0]
        }
        "p" => style.margin = [0.0, 0.0, 9.0, 0.0],
        "strong" | "b" | "th" => style.weight = 700,
        "em" | "i" => style.font_style = "italic".into(),
        "u" | "a" => style.text_decoration = "underline".into(),
        "sup" | "sub" => {
            style.font_size *= 0.8;
            style.line_height *= 0.8;
            style.vertical_align = if tag == "sup" { "super" } else { "sub" }.into();
        }
        "table" => style.display = "table".into(),
        "colgroup" => style.display = "table-column-group".into(),
        "col" => style.display = "table-column".into(),
        "thead" => style.display = "table-header-group".into(),
        "tbody" => style.display = "table-row-group".into(),
        "tfoot" => style.display = "table-footer-group".into(),
        "tr" => style.display = "table-row".into(),
        _ => {}
    }
}

#[derive(Clone, Copy, Default)]
struct TableHints {
    cell_padding: Option<f32>,
    cell_border: Option<f32>,
}

fn html_length(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.ends_with('%') {
        return Some(value.to_owned());
    }
    value
        .parse::<f32>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| format!("{number}px"))
}

fn numeric_hint(value: Option<&str>) -> Option<f32> {
    value?
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number * 0.75)
}

fn empty_dom_subtree(dom: &NodeRef) -> bool {
    dom.children().all(|child| {
        if let Some(text) = child.as_text() {
            text.borrow().trim().is_empty()
        } else if child
            .as_element()
            .is_some_and(|element| element.name.local.as_ref() == "img")
        {
            false
        } else {
            empty_dom_subtree(&child)
        }
    })
}

fn apply_presentational_hints(
    tag: &str,
    attrs: &HashMap<String, String>,
    inherited: TableHints,
    style: &mut Style,
    warnings: &mut Vec<Diagnostic>,
) -> TableHints {
    if matches!(tag, "table" | "colgroup" | "col" | "td" | "th") {
        if let Some(width) = attrs.get("width").and_then(|value| html_length(value)) {
            css::apply(style, "width", &width, warnings);
        }
        if let Some(height) = attrs.get("height").and_then(|value| html_length(value)) {
            css::apply(style, "height", &height, warnings);
        }
    }
    if matches!(tag, "td" | "th") {
        if let Some(padding) = inherited.cell_padding {
            style.padding = [padding; 4];
        }
        if let Some(border) = inherited.cell_border {
            style.border_width = border;
            style.border_widths = [border; 4];
            style.border_styles = std::array::from_fn(|_| "solid".into());
        }
        if let Some(align) = attrs.get("align").map(|value| value.to_ascii_lowercase()) {
            css::apply(style, "text-align", &align, warnings);
        }
        if let Some(align) = attrs.get("valign").map(|value| value.to_ascii_lowercase()) {
            css::apply(style, "vertical-align", &align, warnings);
        }
    }
    if tag != "table" {
        return inherited;
    }
    if let Some(spacing) = attrs
        .get("cellspacing")
        .and_then(|value| html_length(value))
    {
        css::apply(style, "border-spacing", &spacing, warnings);
    }
    if let Some(align) = attrs.get("align").map(|value| value.to_ascii_lowercase()) {
        css::apply(style, "text-align", &align, warnings);
    }
    let cell_border = numeric_hint(attrs.get("border").map(String::as_str));
    if let Some(border) = cell_border {
        style.border_width = border;
        style.border_widths = [border; 4];
        style.border_styles = std::array::from_fn(|_| "solid".into());
    }
    TableHints {
        cell_padding: numeric_hint(attrs.get("cellpadding").map(String::as_str)),
        cell_border,
    }
}

#[allow(clippy::too_many_arguments)]
fn convert(
    dom: &NodeRef,
    parent: &Style,
    sheet: &Sheet,
    warnings: &mut Vec<Diagnostic>,
    presentational_hints: bool,
    inherited_table_hints: TableHints,
    source_offsets: &mut SourceOffsets,
    parent_source_offset: Option<usize>,
) -> Option<Node> {
    if let Some(text) = dom.as_text() {
        return Some(Node {
            tag: "#text".into(),
            attrs: HashMap::new(),
            style: Style::inherit(parent),
            text: text.borrow().to_string(),
            children: vec![],
            source_offset: parent_source_offset,
        });
    }
    let element = dom.as_element()?;
    let tag = element.name.local.to_string();
    let source_offset = source_offsets.take(&tag).or(parent_source_offset);
    if !TAGS.contains(&tag.as_str()) {
        warnings.push(Diagnostic::new(
            "html-tag",
            format!("unsupported HTML tag: <{tag}>"),
        ));
        return None;
    }
    if tag == "head" || tag == "style" {
        return None;
    }
    let attrs: HashMap<String, String> = element
        .attributes
        .borrow()
        .map
        .iter()
        .map(|(k, v)| (k.local.to_string(), v.value.clone()))
        .collect();
    let mut style = Style::inherit(parent);
    defaults(&tag, &mut style);
    let child_table_hints = if presentational_hints {
        apply_presentational_hints(&tag, &attrs, inherited_table_hints, &mut style, warnings)
    } else {
        TableHints::default()
    };
    let matched: Vec<_> = sheet
        .rules
        .iter()
        .filter(|r| css::selector_matches_dom(&r.selector, dom))
        .collect();
    let mut declarations = Vec::new();
    for rule in matched {
        for (key, value) in &rule.declarations {
            declarations.push((
                value.contains("!important"),
                rule.specificity,
                rule.order,
                key.as_str(),
                value.as_str(),
            ));
        }
    }
    let inline_decls = attrs
        .get("style")
        .map(|s| css::declarations(s))
        .unwrap_or_default();
    for (key, value) in &inline_decls {
        declarations.push((
            value.contains("!important"),
            1000,
            usize::MAX,
            key.as_str(),
            value.as_str(),
        ));
    }
    declarations
        .sort_by_key(|(important, specificity, order, _, _)| (*important, *specificity, *order));
    for (_, _, _, key, value) in declarations {
        css::apply(&mut style, key, value, warnings);
    }
    if tag == "img" && (style.border_width > 0.0 || style.background.is_some()) {
        let property = if style.background.is_some() {
            "background"
        } else {
            "border"
        };
        let mut diagnostic = Diagnostic::new(
            "css-property",
            "image borders and backgrounds are unsupported",
        );
        diagnostic.property = Some(property.into());
        warnings.push(diagnostic);
    }
    if style.position != "static"
        && style.position != "running"
        && style.display == "inline"
        && matches!(
            tag.as_str(),
            "span" | "strong" | "b" | "em" | "i" | "u" | "a" | "sup" | "sub" | "br"
        )
        && !(style.height == Some(0.0)
            && style.color_alpha <= f32::EPSILON
            && style.background.is_none()
            && style
                .border_widths
                .iter()
                .all(|width| *width <= f32::EPSILON)
            && empty_dom_subtree(dom))
    {
        warnings.push(
            Diagnostic::new(
                "css-property",
                format!(
                    "position: {} is unsupported on inline <{tag}>",
                    style.position
                ),
            )
            .property("position", &style.position),
        );
    }
    if style.transform != [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        && style.display == "inline"
        && matches!(
            tag.as_str(),
            "span" | "strong" | "b" | "em" | "i" | "u" | "a" | "sup" | "sub" | "br"
        )
    {
        let mut diagnostic = Diagnostic::new(
            "css-property",
            format!("transform is unsupported on inline <{tag}>"),
        );
        diagnostic.property = Some("transform".into());
        warnings.push(diagnostic);
    }
    if style.break_inside_avoid
        && style.display == "block"
        && matches!(
            tag.as_str(),
            "div" | "section" | "article" | "body" | "span" | "td" | "th"
        )
    {
        warnings.push(
            Diagnostic::new(
                "css-property",
                format!("break-inside: avoid is unsupported on <{tag}>"),
            )
            .property("break-inside", "avoid"),
        );
    }
    if (style.break_before || style.break_after)
        && matches!(
            tag.as_str(),
            "span" | "strong" | "b" | "em" | "i" | "td" | "th" | "tr" | "thead" | "tbody" | "tfoot"
        )
    {
        warnings.push(
            Diagnostic::new(
                "css-property",
                format!("page break is unsupported on <{tag}>"),
            )
            .property(
                if style.break_before {
                    "break-before"
                } else {
                    "break-after"
                },
                "page",
            ),
        );
    }
    if style.display == "none" {
        return None;
    }
    let children = dom
        .children()
        .filter_map(|c| {
            convert(
                &c,
                &style,
                sheet,
                warnings,
                presentational_hints,
                child_table_hints,
                source_offsets,
                source_offset,
            )
        })
        .collect();
    Some(Node {
        tag,
        attrs,
        style,
        text: String::new(),
        children,
        source_offset,
    })
}

/// Parse HTML5 syntax and return the visible body subtree plus diagnostics.
pub fn parse(html: &str, sheet: &Sheet, strict: bool) -> Result<(Node, Vec<Diagnostic>)> {
    let (root, warnings, _) = parse_with_metadata(html, sheet, strict)?;
    Ok((root, warnings))
}

pub fn parse_with_metadata(
    html: &str,
    sheet: &Sheet,
    strict: bool,
) -> Result<(Node, Vec<Diagnostic>, DocumentMetadata)> {
    parse_with_metadata_and_hints(html, sheet, strict, false)
}

pub fn parse_with_metadata_and_hints(
    html: &str,
    sheet: &Sheet,
    strict: bool,
    presentational_hints: bool,
) -> Result<(Node, Vec<Diagnostic>, DocumentMetadata)> {
    let dom = kuchiki::parse_html().one(html);
    let mut metadata = DocumentMetadata {
        title: dom
            .select_first("title")
            .ok()
            .map(|n| n.text_contents().trim().to_owned())
            .filter(|v| !v.is_empty()),
        language: dom
            .select_first("html")
            .ok()
            .and_then(|n| n.attributes.borrow().get("lang").map(str::to_owned)),
        ..DocumentMetadata::default()
    };
    if let Ok(metas) = dom.select("meta") {
        for meta in metas {
            let attrs = meta.attributes.borrow();
            if let (Some(name), Some(value)) = (attrs.get("name"), attrs.get("content")) {
                match name.to_ascii_lowercase().as_str() {
                    "author" => metadata.author = Some(value.into()),
                    "subject" | "description" => metadata.subject = Some(value.into()),
                    "keywords" => metadata.keywords = Some(value.into()),
                    _ => {}
                }
            }
        }
    }
    let body = dom
        .select_first("body")
        .map_err(|_| Error("HTML body not found".into()))?;
    let mut warnings = sheet.warnings.clone();
    if dom.select_first("script").is_ok() {
        warnings.push(Diagnostic::new(
            "html-tag",
            "JavaScript <script> is unsupported",
        ));
    }
    let root_style = Style::default();
    let mut source_offsets = SourceOffsets::new(html);
    let root = convert(
        body.as_node(),
        &root_style,
        sheet,
        &mut warnings,
        presentational_hints,
        TableHints::default(),
        &mut source_offsets,
        None,
    )
    .ok_or_else(|| Error("empty HTML body".into()))?;
    let mut anchors = HashSet::new();
    fn collect_anchors(node: &Node, anchors: &mut HashSet<String>) {
        if let Some(id) = node.attr("id").filter(|id| !id.is_empty()) {
            anchors.insert(id.to_owned());
        }
        if node.tag == "a" {
            if let Some(name) = node.attr("name").filter(|name| !name.is_empty()) {
                anchors.insert(name.to_owned());
            }
        }
        for child in &node.children {
            collect_anchors(child, anchors);
        }
    }
    fn check_links(node: &Node, anchors: &HashSet<String>, warnings: &mut Vec<Diagnostic>) {
        if node.tag == "a" {
            if let Some(href) = node.attr("href") {
                let supported = if let Some(id) = href.strip_prefix('#') {
                    anchors.contains(id)
                } else {
                    matches!(
                        href.split_once(':')
                            .map(|(scheme, _)| scheme.to_ascii_lowercase())
                            .as_deref(),
                        Some("http" | "https" | "mailto")
                    )
                };
                if !supported {
                    warnings.push(Diagnostic::new(
                        "pdf-link",
                        format!("unsupported PDF link target: {href}"),
                    ));
                }
            }
        }
        for child in &node.children {
            check_links(child, anchors, warnings);
        }
    }
    collect_anchors(&root, &mut anchors);
    check_links(&root, &anchors, &mut warnings);
    if strict && !warnings.is_empty() {
        return Err(Error(format!(
            "strict mode: {}: {}",
            warnings[0].code, warnings[0].message
        )));
    }
    Ok((root, warnings, metadata))
}

/// Extract document style elements for the same cascade as the CSS argument.
pub fn embedded_css(html: &str) -> String {
    let dom = kuchiki::parse_html().one(html);
    let mut out = String::new();
    if let Ok(styles) = dom.select("style") {
        for s in styles {
            out.push_str(&s.text_contents());
            out.push('\n');
        }
    }
    out
}
