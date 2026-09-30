//! HTML tree extraction and style resolution for the supported document tags.
use crate::css::{self, Sheet, Style};
use crate::{Diagnostic, Error, Result};
use kuchiki::traits::TendrilSink;
use kuchiki::NodeRef;
use std::collections::{HashMap, HashSet};

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
    "thead", "tbody", "tfoot", "tr", "th", "td",
];

fn defaults(tag: &str, style: &mut Style) {
    if matches!(
        tag,
        "span" | "strong" | "b" | "em" | "i" | "u" | "a" | "sup" | "sub" | "br"
    ) {
        style.display = "inline".into();
    }
    if matches!(tag, "td" | "th") {
        style.display = "table-cell".into();
    }
    match tag {
        "h1" => {
            style.font_size = 24.0;
            style.line_height = 28.8;
            style.weight = 700;
            style.margin = [12.0, 0.0, 12.0, 0.0]
        }
        "h2" => {
            style.font_size = 18.0;
            style.line_height = 21.6;
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
        "tr" => style.display = "table-row".into(),
        _ => {}
    }
}

fn convert(
    dom: &NodeRef,
    parent: &Style,
    sheet: &Sheet,
    warnings: &mut Vec<Diagnostic>,
) -> Option<Node> {
    if let Some(text) = dom.as_text() {
        return Some(Node {
            tag: "#text".into(),
            attrs: HashMap::new(),
            style: parent.clone(),
            text: text.borrow().to_string(),
            children: vec![],
        });
    }
    let element = dom.as_element()?;
    let tag = element.name.local.to_string();
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
    if (style.height.is_some() || style.height_percent.is_some())
        && tag != "img"
        && style.display != "flex"
    {
        warnings.push(Diagnostic::new(
            "css-property",
            format!("height is only supported on <img>, not <{tag}>"),
        ));
    }
    if tag == "img" && (style.border_width > 0.0 || style.background.is_some()) {
        warnings.push(Diagnostic::new(
            "css-property",
            "image borders and backgrounds are unsupported",
        ));
    }
    if style.position == "relative"
        && matches!(
            tag.as_str(),
            "span" | "strong" | "b" | "em" | "i" | "u" | "a" | "sup" | "sub" | "br"
        )
    {
        warnings.push(Diagnostic::new(
            "css-property",
            format!("position: relative is unsupported on inline <{tag}>"),
        ));
    }
    if style.break_inside_avoid
        && matches!(
            tag.as_str(),
            "div" | "section" | "article" | "body" | "span" | "td" | "th"
        )
    {
        warnings.push(Diagnostic::new(
            "css-property",
            format!("break-inside: avoid is unsupported on <{tag}>"),
        ));
    }
    if (style.break_before || style.break_after)
        && matches!(
            tag.as_str(),
            "span" | "strong" | "b" | "em" | "i" | "td" | "th" | "tr" | "thead" | "tbody" | "tfoot"
        )
    {
        warnings.push(Diagnostic::new(
            "css-property",
            format!("page break is unsupported on <{tag}>"),
        ));
    }
    if style.display == "none" {
        return None;
    }
    let children = dom
        .children()
        .filter_map(|c| convert(&c, &style, sheet, warnings))
        .collect();
    Some(Node {
        tag,
        attrs,
        style,
        text: String::new(),
        children,
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
    let root = convert(body.as_node(), &root_style, sheet, &mut warnings)
        .ok_or_else(|| Error("empty HTML body".into()))?;
    let mut anchors = HashSet::new();
    fn collect_anchors(node: &Node, anchors: &mut HashSet<String>) {
        if matches!(
            node.tag.as_str(),
            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "img"
        ) {
            if let Some(id) = node.attr("id").filter(|id| !id.is_empty()) {
                anchors.insert(id.to_owned());
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
        return Err(Error(warnings[0].message.clone()));
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
