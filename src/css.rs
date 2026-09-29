//! CSS parsing, a small selector cascade, and inherited document styles.
use crate::{Diagnostic, Error, Result};
use std::collections::HashMap;

/// All geometric values are PDF points. CSS px is 0.75 pt and mm is 72/25.4 pt.
pub fn length(value: &str) -> Option<f32> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    for (unit, scale) in [
        ("px", 0.75),
        ("pt", 1.0),
        ("mm", 72.0 / 25.4),
        ("cm", 72.0 / 2.54),
        ("in", 72.0),
    ] {
        if let Some(number) = value.strip_suffix(unit) {
            return number
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .map(|v| v * scale);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32);

pub fn color(value: &str) -> Option<Color> {
    let v = value.trim().to_ascii_lowercase();
    let hex = match v.as_str() {
        "black" => "#000000",
        "white" => "#ffffff",
        "red" => "#ff0000",
        "blue" => "#0000ff",
        "green" => "#008000",
        "gray" | "grey" => "#808080",
        "yellow" => "#ffff00",
        "transparent" => return None,
        _ => &v,
    };
    let raw = hex.strip_prefix('#')?;
    let full = if raw.len() == 3 {
        raw.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        raw.to_string()
    };
    if full.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(&full, 16).ok()?;
    Some(Color(
        ((n >> 16) & 255) as f32 / 255.0,
        ((n >> 8) & 255) as f32 / 255.0,
        (n & 255) as f32 / 255.0,
    ))
}

/// Physical page size and margins, all expressed in PDF points.
#[derive(Clone, Debug)]
pub struct PageStyle {
    pub width: f32,
    pub height: f32,
    pub margin: [f32; 4],
    pub rotation: i16,
    pub landscape_size_keyword: bool,
    pub boxes: HashMap<String, MarginBox>,
}
#[derive(Clone, Debug)]
pub struct MarginBox {
    pub content: String,
    pub family: Vec<String>,
    pub font_size: f32,
    pub color: Color,
}
impl Default for MarginBox {
    fn default() -> Self {
        Self {
            content: String::new(),
            family: vec!["Noto Sans".into()],
            font_size: 8.0,
            color: Color(0.0, 0.0, 0.0),
        }
    }
}
impl Default for PageStyle {
    fn default() -> Self {
        Self {
            width: 595.28,
            height: 841.89,
            margin: [56.7; 4],
            rotation: 0,
            landscape_size_keyword: false,
            boxes: HashMap::new(),
        }
    }
}

/// Resolved visual properties; only text properties inherit.
#[derive(Clone, Debug)]
pub struct Style {
    pub display: String,
    pub family: Vec<String>,
    pub weight: u16,
    pub font_style: String,
    pub font_size: f32,
    pub line_height: f32,
    pub text_align: String,
    pub color: Color,
    pub background: Option<Color>,
    pub margin: [f32; 4],
    pub padding: [f32; 4],
    pub border_width: f32,
    pub border_color: Color,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub max_width_percent: Option<f32>,
    pub max_height_percent: Option<f32>,
    pub aspect_ratio: Option<f32>,
    pub page_name: Option<String>,
    pub flex_direction: String,
    pub align_items: String,
    pub justify_content: String,
    pub grid_columns: Option<usize>,
    pub break_before: bool,
    pub break_after: bool,
    pub break_inside_avoid: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            display: "block".into(),
            family: vec!["Noto Sans".into()],
            weight: 400,
            font_style: "normal".into(),
            font_size: 12.0,
            line_height: 14.4,
            text_align: "left".into(),
            color: Color(0.0, 0.0, 0.0),
            background: None,
            margin: [0.0; 4],
            padding: [0.0; 4],
            border_width: 0.0,
            border_color: Color(0.0, 0.0, 0.0),
            width: None,
            height: None,
            max_width_percent: None,
            max_height_percent: None,
            aspect_ratio: None,
            page_name: None,
            flex_direction: "row".into(),
            align_items: "stretch".into(),
            justify_content: "start".into(),
            grid_columns: None,
            break_before: false,
            break_after: false,
            break_inside_avoid: false,
        }
    }
}
impl Style {
    pub fn inherit(parent: &Style) -> Self {
        Self {
            family: parent.family.clone(),
            weight: parent.weight,
            font_style: parent.font_style.clone(),
            font_size: parent.font_size,
            line_height: parent.line_height,
            text_align: parent.text_align.clone(),
            color: parent.color,
            page_name: parent.page_name.clone(),
            ..Self::default()
        }
    }
}

/// One supported selector and its ordered declarations.
#[derive(Clone, Debug)]
pub struct Rule {
    pub selector: String,
    pub specificity: u32,
    pub order: usize,
    pub declarations: Vec<(String, String)>,
}
/// A local font file declaration imported during layout.
#[derive(Clone, Debug)]
pub struct FontFace {
    pub family: String,
    pub src: String,
    pub weight: u16,
    pub style: String,
}
/// Parsed page rules, element rules, font faces and diagnostics.
#[derive(Clone, Debug)]
pub struct Sheet {
    pub page: PageStyle,
    pub named_pages: HashMap<String, PageStyle>,
    pub first_boxes: HashMap<String, MarginBox>,
    margin_rules: Vec<MarginRule>,
    pub rules: Vec<Rule>,
    pub faces: Vec<FontFace>,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
struct MarginRule {
    page_name: Option<String>,
    first: bool,
    box_name: String,
    declarations: Vec<(String, String)>,
}

impl Sheet {
    /// Resolve page-margin declarations by importance, page-selector specificity,
    /// and source order. A named page outranks `:first` at equal importance.
    pub fn margin_boxes_for(&self, name: Option<&str>, first: bool) -> HashMap<String, MarginBox> {
        type Priority = (bool, u8, u8, usize, usize);
        let mut winners: HashMap<(String, String), (Priority, &str)> = HashMap::new();
        for (rule_index, rule) in self.margin_rules.iter().enumerate() {
            if rule.first && !first {
                continue;
            }
            if let Some(page_name) = &rule.page_name {
                if Some(page_name.as_str()) != name {
                    continue;
                }
            }
            for (declaration_index, (property, raw)) in rule.declarations.iter().enumerate() {
                let value = value_without_important(raw);
                let valid = match property.as_str() {
                    "content" | "font-family" => true,
                    "font-size" => length(value).is_some(),
                    "color" => color(value).is_some(),
                    _ => false,
                };
                if !valid {
                    continue;
                }
                let priority = (
                    raw.trim().ends_with("!important"),
                    u8::from(rule.page_name.is_some()),
                    u8::from(rule.first),
                    rule_index,
                    declaration_index,
                );
                let key = (rule.box_name.clone(), property.clone());
                if winners.get(&key).is_none_or(|(old, _)| priority >= *old) {
                    winners.insert(key, (priority, raw));
                }
            }
        }
        let mut boxes = HashMap::new();
        for ((box_name, property), (_, raw)) in winners {
            let box_style: &mut MarginBox = boxes.entry(box_name).or_default();
            let value = value_without_important(raw);
            match property.as_str() {
                "content" => box_style.content = value.into(),
                "font-size" => {
                    if let Some(size) = length(value) {
                        box_style.font_size = size;
                    }
                }
                "font-family" => {
                    box_style.family = value
                        .split(',')
                        .map(|s| s.trim().trim_matches(['\'', '"']).into())
                        .collect();
                }
                "color" => {
                    if let Some(value) = color(value) {
                        box_style.color = value;
                    }
                }
                _ => {}
            }
        }
        boxes
    }
}

pub fn declarations(body: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    for (i, ch) in body.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if Some(ch) == quote {
            quote = None;
            continue;
        }
        if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        if ch == ';' && quote.is_none() {
            if let Some((k, v)) = body[start..i].split_once(':') {
                result.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
            start = i + 1;
        }
    }
    if let Some((k, v)) = body[start..].split_once(':') {
        result.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    result
}
pub fn value_without_important(value: &str) -> &str {
    value
        .trim()
        .strip_suffix("!important")
        .unwrap_or(value)
        .trim()
}
fn box_values(value: &str) -> Option<[f32; 4]> {
    let v: Vec<f32> = value
        .split_whitespace()
        .map(length)
        .collect::<Option<Vec<_>>>()?;
    match v.as_slice() {
        [a] => Some([*a; 4]),
        [a, b] => Some([*a, *b, *a, *b]),
        [a, b, c] => Some([*a, *b, *c, *b]),
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}
fn unsupported(warnings: &mut Vec<Diagnostic>, code: &'static str, msg: String) {
    warnings.push(Diagnostic { code, message: msg });
}
fn valid_selector(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '-' | '_' | '.' | '#' | '[' | ']' | '=' | '"' | '\'' | ' ' | '>'
                )
        })
}
fn specificity(s: &str) -> u32 {
    (s.matches('#').count() as u32) * 100
        + ((s.matches('.').count() + s.matches('[').count()) as u32) * 10
        + u32::from(s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
}

fn blocks(input: &str) -> Result<(Vec<(&str, &str)>, String)> {
    let mut found = Vec::new();
    let mut loose = String::new();
    let mut pos = 0;
    while pos < input.len() {
        let mut quote = None;
        let mut escaped = false;
        let mut open = None;
        for (offset, ch) in input[pos..].char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote.is_some() {
                escaped = true;
                continue;
            }
            if Some(ch) == quote {
                quote = None;
                continue;
            }
            if quote.is_none() && matches!(ch, '\'' | '"') {
                quote = Some(ch);
                continue;
            }
            if ch == '{' && quote.is_none() {
                open = Some(pos + offset);
                break;
            }
        }
        let Some(open) = open else {
            loose.push_str(&input[pos..]);
            break;
        };
        let prefix = &input[pos..open];
        // A page rule may contain declarations before and between margin boxes.
        let split = prefix.rfind(';').map(|n| n + 1).unwrap_or(0);
        loose.push_str(&prefix[..split]);
        let header = prefix[split..].trim();
        let mut depth = 1;
        let mut quote = None;
        let mut escaped = false;
        let mut close = None;
        for (offset, ch) in input[open + 1..].char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote.is_some() {
                escaped = true;
                continue;
            }
            if Some(ch) == quote {
                quote = None;
                continue;
            }
            if quote.is_none() && matches!(ch, '\'' | '"') {
                quote = Some(ch);
                continue;
            }
            if quote.is_some() {
                continue;
            }
            if ch == '{' {
                depth += 1;
            }
            if ch == '}' {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + 1 + offset);
                    break;
                }
            }
        }
        let close = close.ok_or_else(|| Error(format!("unclosed CSS rule: {header}")))?;
        found.push((header, &input[open + 1..close]));
        pos = close + 1;
    }
    Ok((found, loose))
}

fn parse_page_declarations(
    page: &mut PageStyle,
    decl: Vec<(String, String)>,
    warnings: &mut Vec<Diagnostic>,
) {
    for (key, raw) in decl {
        let value = value_without_important(&raw);
        match key.as_str() {
            "size" => {
                let parts: Vec<_> = value.split_whitespace().collect();
                let size = match parts.as_slice() {
                    ["A4"] | ["a4"] => Some((595.28, 841.89, false)),
                    ["landscape"] => Some((841.89, 595.28, true)),
                    ["portrait"] => Some((595.28, 841.89, false)),
                    ["A4", "landscape"] | ["a4", "landscape"] => Some((841.89, 595.28, true)),
                    [w, h] => length(w).zip(length(h)).map(|(w, h)| (w, h, false)),
                    _ => None,
                };
                if let Some((w, h, landscape_keyword)) =
                    size.filter(|(w, h, _)| *w > 0.0 && *h > 0.0)
                {
                    page.width = w;
                    page.height = h;
                    page.landscape_size_keyword = landscape_keyword;
                } else {
                    unsupported(
                        warnings,
                        "css-value",
                        format!("unsupported @page size: {value}"),
                    );
                }
            }
            "margin" => {
                if let Some(v) = box_values(value) {
                    page.margin = v;
                } else {
                    unsupported(
                        warnings,
                        "css-value",
                        format!("invalid @page margin: {value}"),
                    );
                }
            }
            "page-orientation" if value == "rotate-right" => page.rotation = 90,
            "page-orientation" if value == "rotate-left" => page.rotation = 270,
            "page-orientation" if value == "upright" => page.rotation = 0,
            _ => unsupported(
                warnings,
                "css-property",
                format!("unsupported @page property: {key}"),
            ),
        }
    }
}
fn parse_margin_box(body: &str, warnings: &mut Vec<Diagnostic>) -> MarginBox {
    let mut box_style = MarginBox::default();
    for (key, raw) in declarations(body) {
        let value = value_without_important(&raw);
        match key.as_str() {
            "content" => box_style.content = value.into(),
            "font-size" => {
                if let Some(v) = length(value) {
                    box_style.font_size = v;
                } else {
                    unsupported(
                        warnings,
                        "css-value",
                        format!("invalid margin box font-size: {value}"),
                    );
                }
            }
            "font-family" => {
                box_style.family = value
                    .split(',')
                    .map(|s| s.trim().trim_matches(['\'', '"']).into())
                    .collect()
            }
            "color" => {
                if let Some(v) = color(value) {
                    box_style.color = v;
                } else {
                    unsupported(
                        warnings,
                        "css-value",
                        format!("invalid margin box color: {value}"),
                    );
                }
            }
            _ => unsupported(
                warnings,
                "css-property",
                format!("unsupported margin box property: {key}"),
            ),
        }
    }
    box_style
}
fn parse_rules(input: &str, sheet: &mut Sheet) -> Result<()> {
    let (found, loose) = blocks(input)?;
    if !loose.trim().is_empty() {
        unsupported(
            &mut sheet.warnings,
            "css-syntax",
            format!("trailing CSS: {}", loose.trim()),
        );
    }
    for (selector, body) in found {
        if selector == "@media print" {
            parse_rules(body, sheet)?;
        } else if selector.starts_with("@page") {
            let name = selector.trim_start_matches("@page").trim();
            if name.starts_with(':') && name != ":first" {
                unsupported(
                    &mut sheet.warnings,
                    "at-rule",
                    format!("unsupported @page selector: {name}"),
                );
                continue;
            }
            let mut page = if name.is_empty() || name == ":first" {
                sheet.page.clone()
            } else {
                sheet
                    .named_pages
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| sheet.page.clone())
            };
            let (nested, plain) = blocks(body)?;
            if name == ":first" && !plain.trim().is_empty() {
                unsupported(
                    &mut sheet.warnings,
                    "css-property",
                    "@page :first supports margin boxes only".into(),
                );
            }
            parse_page_declarations(&mut page, declarations(&plain), &mut sheet.warnings);
            for (box_name, box_body) in nested {
                if matches!(
                    box_name,
                    "@bottom-left"
                        | "@bottom-center"
                        | "@bottom-right"
                        | "@top-left"
                        | "@top-center"
                        | "@top-right"
                ) {
                    let box_style = parse_margin_box(box_body, &mut sheet.warnings);
                    sheet.margin_rules.push(MarginRule {
                        page_name: if name.is_empty() || name == ":first" {
                            None
                        } else {
                            Some(name.into())
                        },
                        first: name == ":first",
                        box_name: box_name.into(),
                        declarations: declarations(box_body),
                    });
                    if name == ":first" {
                        sheet.first_boxes.insert(box_name.into(), box_style);
                    } else {
                        page.boxes.insert(box_name.into(), box_style);
                    }
                } else {
                    unsupported(
                        &mut sheet.warnings,
                        "at-rule",
                        format!("unsupported page at-rule: {box_name}"),
                    );
                }
            }
            if name.is_empty() {
                sheet.page = page;
            } else if name != ":first" {
                sheet.named_pages.insert(name.into(), page);
            }
        } else if selector == "@font-face" {
            let map: HashMap<_, _> = declarations(body).into_iter().collect();
            if let (Some(family), Some(src)) = (map.get("font-family"), map.get("src")) {
                sheet.faces.push(FontFace {
                    family: family.trim_matches(['\'', '"']).into(),
                    src: src.clone(),
                    weight: map
                        .get("font-weight")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(400),
                    style: map
                        .get("font-style")
                        .cloned()
                        .unwrap_or_else(|| "normal".into()),
                });
            } else {
                unsupported(
                    &mut sheet.warnings,
                    "font-face",
                    "@font-face needs font-family and src".into(),
                );
            }
        } else if selector.starts_with('@') {
            unsupported(
                &mut sheet.warnings,
                "at-rule",
                format!("unsupported at-rule: {selector}"),
            );
        } else {
            let decl = declarations(body);
            for part in selector.split(',').map(str::trim) {
                if !valid_selector(part) {
                    unsupported(
                        &mut sheet.warnings,
                        "selector",
                        format!("unsupported selector: {part}"),
                    );
                    continue;
                }
                let order = sheet.rules.len();
                sheet.rules.push(Rule {
                    selector: part.into(),
                    specificity: specificity(part),
                    order,
                    declarations: decl.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Parse the supported print stylesheet, preserving unsupported constructs as diagnostics.
pub fn parse(css: &str, strict: bool) -> Result<Sheet> {
    let mut clean = String::new();
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        clean.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail
            .find("*/")
            .ok_or_else(|| Error("unclosed CSS comment".into()))?;
        rest = &tail[end + 2..];
    }
    clean.push_str(rest);
    let mut sheet = Sheet {
        page: PageStyle::default(),
        named_pages: HashMap::new(),
        first_boxes: HashMap::new(),
        margin_rules: vec![],
        rules: vec![],
        faces: vec![],
        warnings: vec![],
    };
    parse_rules(&clean, &mut sheet)?;
    if strict && !sheet.warnings.is_empty() {
        return Err(Error(sheet.warnings[0].message.clone()));
    }
    Ok(sheet)
}

pub fn selector_matches(selector: &str, tag: &str, id: Option<&str>, classes: &str) -> bool {
    let mut remaining = selector;
    let tag_end = remaining.find(['.', '#']).unwrap_or(remaining.len());
    let wanted_tag = &remaining[..tag_end];
    if !wanted_tag.is_empty() && wanted_tag != tag {
        return false;
    }
    remaining = &remaining[tag_end..];
    while !remaining.is_empty() {
        let marker = remaining.as_bytes()[0] as char;
        remaining = &remaining[1..];
        let end = remaining.find(['.', '#']).unwrap_or(remaining.len());
        let name = &remaining[..end];
        if marker == '#' && id != Some(name) {
            return false;
        }
        if marker == '.' && !classes.split_whitespace().any(|c| c == name) {
            return false;
        }
        remaining = &remaining[end..];
    }
    true
}

fn simple_matches(selector: &str, dom: &kuchiki::NodeRef) -> bool {
    let Some(element) = dom.as_element() else {
        return false;
    };
    let attrs = element.attributes.borrow();
    let mut simple = selector.trim();
    if let Some(start) = simple.find('[') {
        let Some(end) = simple[start..].find(']').map(|n| start + n) else {
            return false;
        };
        let condition = &simple[start + 1..end];
        let Some((key, expected)) = condition.split_once('=') else {
            return false;
        };
        if attrs.get(key.trim()) != Some(expected.trim().trim_matches(['\'', '"'])) {
            return false;
        }
        simple = &simple[..start];
    }
    selector_matches(
        simple,
        element.name.local.as_ref(),
        attrs.get("id"),
        attrs.get("class").unwrap_or(""),
    )
}
pub fn selector_matches_dom(selector: &str, dom: &kuchiki::NodeRef) -> bool {
    if let Some((ancestor, target)) = selector.rsplit_once('>') {
        return simple_matches(target, dom)
            && dom
                .parent()
                .is_some_and(|p| selector_matches_dom(ancestor.trim(), &p));
    }
    // Whitespace means a descendant combinator; the rightmost simple selector
    // matches the element itself, then one of its ancestors matches the rest.
    if let Some((ancestor, target)) = selector.rsplit_once(' ') {
        if !simple_matches(target, dom) {
            return false;
        }
        let mut parent = dom.parent();
        while let Some(p) = parent {
            if selector_matches_dom(ancestor.trim(), &p) {
                return true;
            }
            parent = p.parent();
        }
        return false;
    }
    simple_matches(selector, dom)
}

pub fn apply(style: &mut Style, key: &str, value: &str, warnings: &mut Vec<Diagnostic>) {
    let v = value_without_important(value);
    let mut bad = false;
    match key {
        "display" if matches!(v, "none" | "block" | "inline" | "flex" | "grid") => {
            style.display = v.into()
        }
        "page" if v == "auto" => style.page_name = None,
        "page" if !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            style.page_name = Some(v.into())
        }
        "flex-direction" if matches!(v, "column" | "row") => style.flex_direction = v.into(),
        "align-items" if matches!(v, "center" | "start" | "stretch") => {
            style.align_items = v.into()
        }
        "justify-content" if matches!(v, "center" | "start") => style.justify_content = v.into(),
        "grid-template-columns" if v.starts_with("repeat(") && v.ends_with(", 1fr)") => {
            if let Ok(n) = v[7..v.len() - 6].trim().parse::<usize>() {
                if n > 0 {
                    style.grid_columns = Some(n);
                } else {
                    bad = true;
                }
            } else {
                bad = true;
            }
        }
        "font-family" => {
            style.family = v
                .split(',')
                .map(|s| s.trim().trim_matches(['\'', '"']).to_string())
                .collect()
        }
        "font-weight" => {
            if let Ok(n) = v.parse::<u16>() {
                style.weight = n
            } else if v == "bold" {
                style.weight = 700
            } else if v == "normal" {
                style.weight = 400
            } else {
                bad = true
            }
        }
        "font-style" if matches!(v, "normal" | "italic") => style.font_style = v.into(),
        "font-size" => {
            if let Some(n) = length(v) {
                style.font_size = n;
                style.line_height = n * 1.2
            } else {
                bad = true
            }
        }
        "line-height" => {
            if let Some(n) = length(v) {
                style.line_height = n
            } else if let Ok(n) = v.parse::<f32>() {
                style.line_height = style.font_size * n
            } else {
                bad = true
            }
        }
        "text-align" if matches!(v, "left" | "center" | "right") => style.text_align = v.into(),
        "color" => {
            if let Some(c) = color(v) {
                style.color = c
            } else {
                bad = true
            }
        }
        "background" | "background-color" => {
            if v == "transparent" {
                style.background = None
            } else if let Some(c) = color(v) {
                style.background = Some(c)
            } else {
                bad = true
            }
        }
        "margin" => {
            if let Some(n) = box_values(v) {
                style.margin = n
            } else {
                bad = true
            }
        }
        "padding" => {
            if let Some(n) = box_values(v) {
                style.padding = n
            } else {
                bad = true
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
        | "padding-right" | "padding-bottom" | "padding-left" => {
            if let Some(n) = length(v) {
                let idx = if key.ends_with("top") {
                    0
                } else if key.ends_with("right") {
                    1
                } else if key.ends_with("bottom") {
                    2
                } else {
                    3
                };
                if key.starts_with("margin") {
                    style.margin[idx] = n
                } else {
                    style.padding[idx] = n
                }
            } else {
                bad = true
            }
        }
        "border" => {
            let parts: Vec<&str> = v.split_whitespace().collect();
            if parts.len() >= 2
                && parts.len() <= 3
                && parts[1] == "solid"
                && parts.get(2).is_none_or(|c| color(c).is_some())
                && parts.first().and_then(|s| length(s)).is_some()
            {
                let n = length(parts[0]).unwrap_or(0.0);
                style.border_width = n;
                if let Some(c) = parts.get(2).and_then(|s| color(s)) {
                    style.border_color = c;
                }
            } else {
                bad = true
            }
        }
        "border-width" => {
            if let Some(n) = length(v) {
                style.border_width = n
            } else {
                bad = true
            }
        }
        "border-color" => {
            if let Some(c) = color(v) {
                style.border_color = c
            } else {
                bad = true
            }
        }
        "width" => {
            if let Some(n) = length(v) {
                style.width = Some(n)
            } else {
                bad = true
            }
        }
        "height" => {
            if v == "auto" {
                style.height = None
            } else if let Some(n) = length(v) {
                style.height = Some(n)
            } else {
                bad = true
            }
        }
        "max-width" if v.ends_with('%') => {
            style.max_width_percent = v[..v.len() - 1]
                .parse::<f32>()
                .ok()
                .filter(|n| (0.0..=100.0).contains(n))
                .map(|n| n / 100.0);
            bad = style.max_width_percent.is_none();
        }
        "max-height" if v.ends_with('%') => {
            style.max_height_percent = v[..v.len() - 1]
                .parse::<f32>()
                .ok()
                .filter(|n| (0.0..=100.0).contains(n))
                .map(|n| n / 100.0);
            bad = style.max_height_percent.is_none();
        }
        "aspect-ratio" => {
            style.aspect_ratio = v.parse::<f32>().ok().filter(|n| n.is_finite() && *n > 0.0);
            bad = style.aspect_ratio.is_none();
        }
        "break-before" | "page-break-before" if v == "page" || v == "always" => {
            style.break_before = true
        }
        "break-after" | "page-break-after" if v == "page" || v == "always" => {
            style.break_after = true
        }
        "break-inside" | "page-break-inside" if v == "avoid" => style.break_inside_avoid = true,
        "break-before" | "break-after" | "break-inside" | "page-break-before"
        | "page-break-after" | "page-break-inside"
            if v == "auto" => {}
        _ => bad = true,
    }
    if bad {
        unsupported(
            warnings,
            "css-property",
            format!("unsupported CSS declaration: {key}: {v}"),
        );
    }
}
