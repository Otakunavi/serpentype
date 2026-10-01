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

/// Resolve a length using the current font size; `rem` uses the 12pt root default.
pub fn computed_length(value: &str, font_size: f32) -> Option<f32> {
    let value = value.trim();
    if let Some(inner) = value
        .strip_prefix("calc(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split_whitespace().collect();
        if parts.len() != 3 {
            return None;
        }
        let left = computed_length(parts[0], font_size)?;
        let right = computed_length(parts[2], font_size)?;
        return match parts[1] {
            "+" => Some(left + right),
            "-" => Some(left - right),
            _ => None,
        }
        .filter(|v| v.is_finite());
    }
    if let Some(number) = value.strip_suffix("rem") {
        return number
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .map(|v| v * 12.0);
    }
    if let Some(number) = value.strip_suffix("em") {
        return number
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .map(|v| v * font_size);
    }
    length(value)
}
fn border_length(value: &str, font_size: f32) -> Option<f32> {
    match value {
        "thin" => Some(0.75),
        "medium" => Some(2.25),
        "thick" => Some(3.75),
        _ => computed_length(value, font_size),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32);

fn named_color(value: &str) -> Option<&'static str> {
    Some(match value {
        "aliceblue" => "#f0f8ff",
        "antiquewhite" => "#faebd7",
        "aqua" => "#00ffff",
        "aquamarine" => "#7fffd4",
        "azure" => "#f0ffff",
        "beige" => "#f5f5dc",
        "bisque" => "#ffe4c4",
        "black" => "#000000",
        "blanchedalmond" => "#ffebcd",
        "blue" => "#0000ff",
        "blueviolet" => "#8a2be2",
        "brown" => "#a52a2a",
        "burlywood" => "#deb887",
        "cadetblue" => "#5f9ea0",
        "chartreuse" => "#7fff00",
        "chocolate" => "#d2691e",
        "coral" => "#ff7f50",
        "cornflowerblue" => "#6495ed",
        "cornsilk" => "#fff8dc",
        "crimson" => "#dc143c",
        "cyan" => "#00ffff",
        "darkblue" => "#00008b",
        "darkcyan" => "#008b8b",
        "darkgoldenrod" => "#b8860b",
        "darkgray" => "#a9a9a9",
        "darkgreen" => "#006400",
        "darkgrey" => "#a9a9a9",
        "darkkhaki" => "#bdb76b",
        "darkmagenta" => "#8b008b",
        "darkolivegreen" => "#556b2f",
        "darkorange" => "#ff8c00",
        "darkorchid" => "#9932cc",
        "darkred" => "#8b0000",
        "darksalmon" => "#e9967a",
        "darkseagreen" => "#8fbc8f",
        "darkslateblue" => "#483d8b",
        "darkslategray" => "#2f4f4f",
        "darkslategrey" => "#2f4f4f",
        "darkturquoise" => "#00ced1",
        "darkviolet" => "#9400d3",
        "deeppink" => "#ff1493",
        "deepskyblue" => "#00bfff",
        "dimgray" => "#696969",
        "dimgrey" => "#696969",
        "dodgerblue" => "#1e90ff",
        "firebrick" => "#b22222",
        "floralwhite" => "#fffaf0",
        "forestgreen" => "#228b22",
        "fuchsia" => "#ff00ff",
        "gainsboro" => "#dcdcdc",
        "ghostwhite" => "#f8f8ff",
        "gold" => "#ffd700",
        "goldenrod" => "#daa520",
        "gray" => "#808080",
        "green" => "#008000",
        "greenyellow" => "#adff2f",
        "grey" => "#808080",
        "honeydew" => "#f0fff0",
        "hotpink" => "#ff69b4",
        "indianred" => "#cd5c5c",
        "indigo" => "#4b0082",
        "ivory" => "#fffff0",
        "khaki" => "#f0e68c",
        "lavender" => "#e6e6fa",
        "lavenderblush" => "#fff0f5",
        "lawngreen" => "#7cfc00",
        "lemonchiffon" => "#fffacd",
        "lightblue" => "#add8e6",
        "lightcoral" => "#f08080",
        "lightcyan" => "#e0ffff",
        "lightgoldenrodyellow" => "#fafad2",
        "lightgray" => "#d3d3d3",
        "lightgreen" => "#90ee90",
        "lightgrey" => "#d3d3d3",
        "lightpink" => "#ffb6c1",
        "lightsalmon" => "#ffa07a",
        "lightseagreen" => "#20b2aa",
        "lightskyblue" => "#87cefa",
        "lightslategray" => "#778899",
        "lightslategrey" => "#778899",
        "lightsteelblue" => "#b0c4de",
        "lightyellow" => "#ffffe0",
        "lime" => "#00ff00",
        "limegreen" => "#32cd32",
        "linen" => "#faf0e6",
        "magenta" => "#ff00ff",
        "maroon" => "#800000",
        "mediumaquamarine" => "#66cdaa",
        "mediumblue" => "#0000cd",
        "mediumorchid" => "#ba55d3",
        "mediumpurple" => "#9370db",
        "mediumseagreen" => "#3cb371",
        "mediumslateblue" => "#7b68ee",
        "mediumspringgreen" => "#00fa9a",
        "mediumturquoise" => "#48d1cc",
        "mediumvioletred" => "#c71585",
        "midnightblue" => "#191970",
        "mintcream" => "#f5fffa",
        "mistyrose" => "#ffe4e1",
        "moccasin" => "#ffe4b5",
        "navajowhite" => "#ffdead",
        "navy" => "#000080",
        "oldlace" => "#fdf5e6",
        "olive" => "#808000",
        "olivedrab" => "#6b8e23",
        "orange" => "#ffa500",
        "orangered" => "#ff4500",
        "orchid" => "#da70d6",
        "palegoldenrod" => "#eee8aa",
        "palegreen" => "#98fb98",
        "paleturquoise" => "#afeeee",
        "palevioletred" => "#db7093",
        "papayawhip" => "#ffefd5",
        "peachpuff" => "#ffdab9",
        "peru" => "#cd853f",
        "pink" => "#ffc0cb",
        "plum" => "#dda0dd",
        "powderblue" => "#b0e0e6",
        "purple" => "#800080",
        "rebeccapurple" => "#663399",
        "red" => "#ff0000",
        "rosybrown" => "#bc8f8f",
        "royalblue" => "#4169e1",
        "saddlebrown" => "#8b4513",
        "salmon" => "#fa8072",
        "sandybrown" => "#f4a460",
        "seagreen" => "#2e8b57",
        "seashell" => "#fff5ee",
        "sienna" => "#a0522d",
        "silver" => "#c0c0c0",
        "skyblue" => "#87ceeb",
        "slateblue" => "#6a5acd",
        "slategray" => "#708090",
        "slategrey" => "#708090",
        "snow" => "#fffafa",
        "springgreen" => "#00ff7f",
        "steelblue" => "#4682b4",
        "tan" => "#d2b48c",
        "teal" => "#008080",
        "thistle" => "#d8bfd8",
        "tomato" => "#ff6347",
        "turquoise" => "#40e0d0",
        "violet" => "#ee82ee",
        "wheat" => "#f5deb3",
        "white" => "#ffffff",
        "whitesmoke" => "#f5f5f5",
        "yellow" => "#ffff00",
        "yellowgreen" => "#9acd32",
        _ => return None,
    })
}
fn finite_f32(raw: &str) -> Option<f32> {
    raw.parse::<f32>().ok().filter(|value| value.is_finite())
}

pub fn color_with_alpha(value: &str) -> Option<(Color, f32)> {
    let v = value.trim().to_ascii_lowercase();
    if v == "transparent" {
        return Some((Color(0.0, 0.0, 0.0), 0.0));
    }
    if let Some(body) = v
        .strip_prefix("rgb(")
        .or_else(|| v.strip_prefix("rgba("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = body
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let alpha = if parts.len() == 4 {
            if let Some(percent) = parts[3].strip_suffix('%') {
                finite_f32(percent)?.clamp(0.0, 100.0) / 100.0
            } else {
                finite_f32(parts[3])?.clamp(0.0, 1.0)
            }
        } else {
            1.0
        };
        let channel = |raw: &str| -> Option<f32> {
            if let Some(p) = raw.strip_suffix('%') {
                Some(finite_f32(p)?.clamp(0.0, 100.0) / 100.0)
            } else {
                Some(finite_f32(raw)?.clamp(0.0, 255.0) / 255.0)
            }
        };
        return Some((
            Color(channel(parts[0])?, channel(parts[1])?, channel(parts[2])?),
            alpha,
        ));
    }
    if let Some(body) = v
        .strip_prefix("hsl(")
        .or_else(|| v.strip_prefix("hsla("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = body
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let alpha = if parts.len() == 4 {
            if let Some(percent) = parts[3].strip_suffix('%') {
                finite_f32(percent)?.clamp(0.0, 100.0) / 100.0
            } else {
                finite_f32(parts[3])?.clamp(0.0, 1.0)
            }
        } else {
            1.0
        };
        let hue = finite_f32(parts[0].trim_end_matches("deg"))?.rem_euclid(360.0);
        let saturation = finite_f32(parts[1].strip_suffix('%')?)?.clamp(0.0, 100.0) / 100.0;
        let lightness = finite_f32(parts[2].strip_suffix('%')?)?.clamp(0.0, 100.0) / 100.0;
        let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
        let segment = hue / 60.0;
        let secondary = chroma * (1.0 - (segment.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match segment as u8 {
            0 => (chroma, secondary, 0.0),
            1 => (secondary, chroma, 0.0),
            2 => (0.0, chroma, secondary),
            3 => (0.0, secondary, chroma),
            4 => (secondary, 0.0, chroma),
            _ => (chroma, 0.0, secondary),
        };
        let m = lightness - chroma / 2.0;
        return Some((Color(r + m, g + m, b + m), alpha));
    }
    let hex = named_color(&v).unwrap_or(&v);
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
    Some((
        Color(
            ((n >> 16) & 255) as f32 / 255.0,
            ((n >> 8) & 255) as f32 / 255.0,
            (n & 255) as f32 / 255.0,
        ),
        1.0,
    ))
}
pub fn color(value: &str) -> Option<Color> {
    let (color, alpha) = color_with_alpha(value)?;
    (alpha >= 1.0 - f32::EPSILON).then_some(color)
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

/// Resolved visual properties; text properties and visibility inherit.
#[derive(Clone, Debug)]
pub struct Style {
    pub display: String,
    pub visibility: String,
    pub position: String,
    pub inset: [Option<f32>; 4],
    pub family: Vec<String>,
    pub weight: u16,
    pub font_style: String,
    pub font_stretch: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub text_align: String,
    pub text_indent: f32,
    pub overflow_wrap: String,
    pub word_break: String,
    pub text_decoration: String,
    pub vertical_align: String,
    pub white_space: String,
    pub hyphens: String,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub color: Color,
    pub color_alpha: f32,
    pub background: Option<Color>,
    pub background_alpha: f32,
    pub opacity: f32,
    pub parent_opacity: f32,
    pub margin: [f32; 4],
    pub margin_percent: [Option<f32>; 4],
    pub margin_top_consumed: bool,
    pub margin_bottom_consumed: bool,
    pub padding: [f32; 4],
    pub padding_percent: [Option<f32>; 4],
    pub border_width: f32,
    pub border_color: Color,
    pub border_widths: [f32; 4],
    pub border_colors: [Color; 4],
    pub border_alphas: [f32; 4],
    pub border_styles: [String; 4],
    pub border_radius: [f32; 4],
    pub overflow: String,
    pub box_sizing: String,
    pub width: Option<f32>,
    pub width_percent: Option<f32>,
    pub min_width: Option<f32>,
    pub min_width_percent: Option<f32>,
    pub max_width: Option<f32>,
    pub height: Option<f32>,
    pub height_percent: Option<f32>,
    pub min_height: Option<f32>,
    pub min_height_percent: Option<f32>,
    pub max_height: Option<f32>,
    pub max_width_percent: Option<f32>,
    pub max_height_percent: Option<f32>,
    pub aspect_ratio: Option<f32>,
    pub page_name: Option<String>,
    pub flex_direction: String,
    pub align_items: String,
    pub justify_content: String,
    pub grid_columns: Option<usize>,
    pub row_gap: f32,
    pub column_gap: f32,
    pub table_layout: String,
    pub border_collapse: String,
    pub border_spacing: [f32; 2],
    pub break_before: bool,
    pub break_after: bool,
    pub break_before_side: Option<String>,
    pub break_after_side: Option<String>,
    pub break_inside_avoid: bool,
    pub orphans: usize,
    pub widows: usize,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            display: "block".into(),
            visibility: "visible".into(),
            position: "static".into(),
            inset: [None; 4],
            family: vec!["Noto Sans".into()],
            weight: 400,
            font_style: "normal".into(),
            font_stretch: 100.0,
            font_size: 12.0,
            line_height: 14.4,
            text_align: "left".into(),
            text_indent: 0.0,
            overflow_wrap: "break-word".into(),
            word_break: "normal".into(),
            text_decoration: "none".into(),
            vertical_align: "baseline".into(),
            white_space: "normal".into(),
            hyphens: "manual".into(),
            letter_spacing: 0.0,
            word_spacing: 0.0,
            color: Color(0.0, 0.0, 0.0),
            color_alpha: 1.0,
            background: None,
            background_alpha: 1.0,
            opacity: 1.0,
            parent_opacity: 1.0,
            margin: [0.0; 4],
            margin_percent: [None; 4],
            margin_top_consumed: false,
            margin_bottom_consumed: false,
            padding: [0.0; 4],
            padding_percent: [None; 4],
            border_width: 0.0,
            border_color: Color(0.0, 0.0, 0.0),
            border_widths: [0.0; 4],
            border_colors: [Color(0.0, 0.0, 0.0); 4],
            border_alphas: [1.0; 4],
            border_styles: std::array::from_fn(|_| "none".into()),
            border_radius: [0.0; 4],
            overflow: "visible".into(),
            box_sizing: "content-box".into(),
            width: None,
            width_percent: None,
            min_width: None,
            min_width_percent: None,
            max_width: None,
            height: None,
            height_percent: None,
            min_height: None,
            min_height_percent: None,
            max_height: None,
            max_width_percent: None,
            max_height_percent: None,
            aspect_ratio: None,
            page_name: None,
            flex_direction: "row".into(),
            align_items: "stretch".into(),
            justify_content: "start".into(),
            grid_columns: None,
            row_gap: 0.0,
            column_gap: 0.0,
            table_layout: "auto".into(),
            border_collapse: "separate".into(),
            border_spacing: [0.0; 2],
            break_before: false,
            break_after: false,
            break_before_side: None,
            break_after_side: None,
            break_inside_avoid: false,
            orphans: 2,
            widows: 2,
        }
    }
}
impl Style {
    pub fn inherit(parent: &Style) -> Self {
        Self {
            visibility: parent.visibility.clone(),
            family: parent.family.clone(),
            weight: parent.weight,
            font_style: parent.font_style.clone(),
            font_stretch: parent.font_stretch,
            font_size: parent.font_size,
            line_height: parent.line_height,
            text_align: parent.text_align.clone(),
            overflow_wrap: parent.overflow_wrap.clone(),
            word_break: parent.word_break.clone(),
            text_decoration: parent.text_decoration.clone(),
            white_space: parent.white_space.clone(),
            hyphens: parent.hyphens.clone(),
            letter_spacing: parent.letter_spacing,
            word_spacing: parent.word_spacing,
            color: parent.color,
            color_alpha: parent.color_alpha,
            // Opacity is represented cumulatively so descendants paint with the
            // same group alpha without changing the public display-list model.
            opacity: parent.opacity,
            parent_opacity: parent.opacity,
            page_name: parent.page_name.clone(),
            orphans: parent.orphans,
            widows: parent.widows,
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
    pub pseudo_pages: HashMap<String, PageStyle>,
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
    pseudo: Option<String>,
    box_name: String,
    declarations: Vec<(String, String)>,
}

impl Sheet {
    /// Resolve page-margin declarations by importance, page-selector specificity,
    /// and source order. A named page outranks `:first` at equal importance.
    pub fn margin_boxes_for(
        &self,
        name: Option<&str>,
        index: usize,
        blank: bool,
    ) -> HashMap<String, MarginBox> {
        type Priority = (bool, u8, u8, usize, usize);
        let mut winners: HashMap<(String, String), (Priority, &str)> = HashMap::new();
        for (rule_index, rule) in self.margin_rules.iter().enumerate() {
            if rule.first && index != 0 {
                continue;
            }
            if let Some(pseudo) = rule.pseudo.as_deref() {
                let applies = match pseudo {
                    ":left" => (index + 1).is_multiple_of(2),
                    ":right" => !(index + 1).is_multiple_of(2),
                    ":blank" => blank,
                    _ => false,
                };
                if !applies {
                    continue;
                }
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
                    if rule.first {
                        2
                    } else if rule.pseudo.as_deref() == Some(":blank") {
                        3
                    } else {
                        u8::from(rule.pseudo.is_some())
                    },
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
fn box_values_ctx(value: &str, font_size: f32) -> Option<[f32; 4]> {
    if value.starts_with("calc(") && value.ends_with(')') {
        return Some([computed_length(value, font_size)?; 4]);
    }
    let values: Vec<f32> = value
        .split_whitespace()
        .map(|v| computed_length(v, font_size))
        .collect::<Option<Vec<_>>>()?;
    match values.as_slice() {
        [a] => Some([*a; 4]),
        [a, b] => Some([*a, *b, *a, *b]),
        [a, b, c] => Some([*a, *b, *c, *b]),
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}
fn edge_values_ctx(value: &str, font_size: f32) -> Option<([f32; 4], [Option<f32>; 4])> {
    let values: Vec<(f32, Option<f32>)> = value
        .split_whitespace()
        .map(|raw| {
            if let Some(percent) = raw.strip_suffix('%').and_then(finite_f32) {
                Some((0.0, Some(percent / 100.0)))
            } else {
                Some((computed_length(raw, font_size)?, None))
            }
        })
        .collect::<Option<_>>()?;
    let expanded = match values.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return None,
    };
    Some((expanded.map(|value| value.0), expanded.map(|value| value.1)))
}
fn border_parts(value: &str, font_size: f32) -> Option<(f32, String, Color, f32)> {
    let mut width = None;
    let mut style = None;
    let mut paint = None;
    for part in value.split_whitespace() {
        if width.is_none() {
            width = border_length(part, font_size).filter(|value| *value >= 0.0);
            if width.is_some() {
                continue;
            }
        }
        if matches!(part, "none" | "solid" | "dashed" | "dotted") {
            style = Some(part.to_string());
            continue;
        }
        if paint.is_none() {
            paint = color_with_alpha(part);
        }
    }
    let style = style?;
    let width = width.unwrap_or(if style == "none" { 0.0 } else { 1.0 });
    let (color, alpha) = paint.unwrap_or((Color(0.0, 0.0, 0.0), 1.0));
    Some((width, style, color, alpha))
}
fn unsupported(warnings: &mut Vec<Diagnostic>, code: &'static str, msg: String) {
    warnings.push(Diagnostic::new(code, msg));
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
            if name.starts_with(':') && !matches!(name, ":first" | ":left" | ":right" | ":blank") {
                unsupported(
                    &mut sheet.warnings,
                    "at-rule",
                    format!("unsupported @page selector: {name}"),
                );
                continue;
            }
            let mut page = if name.is_empty() {
                sheet.page.clone()
            } else if name.starts_with(':') {
                sheet
                    .pseudo_pages
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| sheet.page.clone())
            } else {
                sheet
                    .named_pages
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| sheet.page.clone())
            };
            let (nested, plain) = blocks(body)?;
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
                        page_name: if name.is_empty() || name.starts_with(':') {
                            None
                        } else {
                            Some(name.into())
                        },
                        first: name == ":first",
                        pseudo: matches!(name, ":left" | ":right" | ":blank").then(|| name.into()),
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
            } else if name.starts_with(':') {
                sheet.pseudo_pages.insert(name.into(), page);
            } else {
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
        pseudo_pages: HashMap::new(),
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
        "display"
            if matches!(
                v,
                "none"
                    | "block"
                    | "inline"
                    | "inline-block"
                    | "flex"
                    | "grid"
                    | "table-row-group"
                    | "table-header-group"
                    | "table-footer-group"
            ) =>
        {
            style.display = v.into()
        }
        "visibility" if matches!(v, "visible" | "hidden") => style.visibility = v.into(),
        "overflow" if matches!(v, "visible" | "hidden") => style.overflow = v.into(),
        "box-sizing" if matches!(v, "content-box" | "border-box") => style.box_sizing = v.into(),
        "position" if matches!(v, "static" | "relative") => style.position = v.into(),
        "top" | "right" | "bottom" | "left" => {
            let idx = match key {
                "top" => 0,
                "right" => 1,
                "bottom" => 2,
                _ => 3,
            };
            if v == "auto" {
                style.inset[idx] = None;
            } else if let Some(n) = computed_length(v, style.font_size) {
                style.inset[idx] = Some(n);
            } else {
                bad = true;
            }
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
        "row-gap" | "column-gap" => {
            if let Some(n) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                if key == "row-gap" {
                    style.row_gap = n;
                } else {
                    style.column_gap = n;
                }
            } else {
                bad = true;
            }
        }
        "gap" => {
            let values: Vec<_> = v.split_whitespace().collect();
            if values.len() == 1 || values.len() == 2 {
                let row = computed_length(values[0], style.font_size).filter(|n| *n >= 0.0);
                let column =
                    computed_length(values.get(1).copied().unwrap_or(values[0]), style.font_size)
                        .filter(|n| *n >= 0.0);
                if let (Some(row), Some(column)) = (row, column) {
                    style.row_gap = row;
                    style.column_gap = column;
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
                if (100..=900).contains(&n) {
                    style.weight = n
                } else {
                    bad = true
                }
            } else if v == "bold" {
                style.weight = 700
            } else if v == "normal" {
                style.weight = 400
            } else {
                bad = true
            }
        }
        "font-style" if matches!(v, "normal" | "italic" | "oblique") => style.font_style = v.into(),
        "font-stretch" => {
            let stretch = match v {
                "ultra-condensed" => Some(50.0),
                "extra-condensed" => Some(62.5),
                "condensed" => Some(75.0),
                "semi-condensed" => Some(87.5),
                "normal" => Some(100.0),
                "semi-expanded" => Some(112.5),
                "expanded" => Some(125.0),
                "extra-expanded" => Some(150.0),
                "ultra-expanded" => Some(200.0),
                _ => v
                    .strip_suffix('%')
                    .and_then(finite_f32)
                    .filter(|value| (50.0..=200.0).contains(value)),
            };
            if let Some(stretch) = stretch {
                style.font_stretch = stretch;
            } else {
                bad = true;
            }
        }
        "font-size" => {
            if let Some(n) = computed_length(v, style.font_size).filter(|n| *n > 0.0) {
                style.font_size = n;
                style.line_height = n * 1.2
            } else {
                bad = true
            }
        }
        "line-height" => {
            if let Some(n) = computed_length(v, style.font_size).filter(|n| *n > 0.0) {
                style.line_height = n
            } else if let Some(n) = finite_f32(v).filter(|n| *n > 0.0) {
                style.line_height = style.font_size * n
            } else {
                bad = true
            }
        }
        "text-align" if matches!(v, "left" | "center" | "right" | "justify") => {
            style.text_align = v.into()
        }
        "text-indent" => {
            if let Some(n) = computed_length(v, style.font_size) {
                style.text_indent = n
            } else {
                bad = true
            }
        }
        "overflow-wrap" if matches!(v, "normal" | "break-word" | "anywhere") => {
            style.overflow_wrap = v.into()
        }
        "word-break" if matches!(v, "normal" | "break-all" | "keep-all") => {
            style.word_break = v.into()
        }
        "text-decoration" if matches!(v, "none" | "underline" | "line-through") => {
            style.text_decoration = v.into()
        }
        "vertical-align"
            if matches!(
                v,
                "baseline" | "middle" | "top" | "bottom" | "super" | "sub"
            ) =>
        {
            style.vertical_align = v.into()
        }
        "white-space" if matches!(v, "normal" | "nowrap" | "pre" | "pre-wrap") => {
            style.white_space = v.into()
        }
        "hyphens" if matches!(v, "none" | "manual") => style.hyphens = v.into(),
        "letter-spacing" => {
            if let Some(n) = computed_length(v, style.font_size) {
                style.letter_spacing = n
            } else {
                bad = true
            }
        }
        "word-spacing" => {
            if let Some(n) = computed_length(v, style.font_size) {
                style.word_spacing = n
            } else {
                bad = true
            }
        }
        "color" => {
            if let Some((c, alpha)) = color_with_alpha(v) {
                style.color = c;
                style.color_alpha = alpha;
            } else {
                bad = true
            }
        }
        "background" | "background-color" => {
            if let Some((c, alpha)) = color_with_alpha(v) {
                style.background = Some(c);
                style.background_alpha = alpha;
            } else {
                bad = true
            }
        }
        "opacity" => {
            if let Some(alpha) = finite_f32(v).filter(|value| (0.0..=1.0).contains(value)) {
                style.opacity = style.parent_opacity * alpha;
            } else {
                bad = true;
            }
        }
        "margin" => {
            if let Some((fixed, percent)) = edge_values_ctx(v, style.font_size) {
                style.margin = fixed;
                style.margin_percent = percent;
            } else {
                bad = true
            }
        }
        "padding" => {
            if let Some((fixed, percent)) =
                edge_values_ctx(v, style.font_size).filter(|(values, percentages)| {
                    values.iter().all(|n| *n >= 0.0)
                        && percentages.iter().flatten().all(|n| *n >= 0.0)
                })
            {
                style.padding = fixed;
                style.padding_percent = percent;
            } else {
                bad = true
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
        | "padding-right" | "padding-bottom" | "padding-left" => {
            let parsed = v
                .strip_suffix('%')
                .and_then(finite_f32)
                .map(|value| (0.0, Some(value / 100.0)))
                .or_else(|| computed_length(v, style.font_size).map(|value| (value, None)));
            if let Some((n, percent)) = parsed.filter(|(n, percent)| {
                key.starts_with("margin") || (*n >= 0.0 && percent.is_none_or(|p| p >= 0.0))
            }) {
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
                    style.margin[idx] = n;
                    style.margin_percent[idx] = percent;
                } else {
                    style.padding[idx] = n;
                    style.padding_percent[idx] = percent;
                }
            } else {
                bad = true
            }
        }
        "border" => {
            if let Some((width, border_style, color, alpha)) = border_parts(v, style.font_size) {
                style.border_width = width;
                style.border_color = color;
                style.border_widths = [width; 4];
                style.border_colors = [color; 4];
                style.border_alphas = [alpha; 4];
                style.border_styles = std::array::from_fn(|_| border_style.clone());
            } else {
                bad = true
            }
        }
        "border-width" => {
            let values: Vec<_> = v.split_whitespace().collect();
            let parsed = values
                .iter()
                .map(|raw| border_length(raw, style.font_size))
                .collect::<Option<Vec<_>>>();
            if let Some(values) = parsed.filter(|values| values.iter().all(|n| *n >= 0.0)) {
                let edges = match values.as_slice() {
                    [a] => Some([*a; 4]),
                    [a, b] => Some([*a, *b, *a, *b]),
                    [a, b, c] => Some([*a, *b, *c, *b]),
                    [a, b, c, d] => Some([*a, *b, *c, *d]),
                    _ => None,
                };
                if let Some(edges) = edges {
                    style.border_widths = edges;
                    style.border_width = edges.into_iter().fold(0.0, f32::max);
                } else {
                    bad = true;
                }
            } else {
                bad = true
            }
        }
        "border-color" => {
            let values: Vec<_> = v
                .split_whitespace()
                .map(color_with_alpha)
                .collect::<Option<_>>()
                .unwrap_or_default();
            let edges = match values.as_slice() {
                [a] => Some([*a; 4]),
                [a, b] => Some([*a, *b, *a, *b]),
                [a, b, c] => Some([*a, *b, *c, *b]),
                [a, b, c, d] => Some([*a, *b, *c, *d]),
                _ => None,
            };
            if let Some(edges) = edges {
                style.border_colors = edges.map(|v| v.0);
                style.border_alphas = edges.map(|v| v.1);
                style.border_color = edges[0].0;
            } else {
                bad = true
            }
        }
        "border-style" => {
            let values: Vec<_> = v.split_whitespace().collect();
            if values
                .iter()
                .all(|value| matches!(*value, "none" | "solid" | "dashed" | "dotted"))
            {
                style.border_styles = match values.as_slice() {
                    [a] => std::array::from_fn(|_| (*a).into()),
                    [a, b] => [(*a).into(), (*b).into(), (*a).into(), (*b).into()],
                    [a, b, c] => [(*a).into(), (*b).into(), (*c).into(), (*b).into()],
                    [a, b, c, d] => [(*a).into(), (*b).into(), (*c).into(), (*d).into()],
                    _ => {
                        bad = true;
                        std::array::from_fn(|_| "none".into())
                    }
                };
            } else {
                bad = true;
            }
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let index = match key {
                "border-top" => 0,
                "border-right" => 1,
                "border-bottom" => 2,
                _ => 3,
            };
            if let Some((width, border_style, color, alpha)) = border_parts(v, style.font_size) {
                style.border_widths[index] = width;
                style.border_styles[index] = border_style;
                style.border_colors[index] = color;
                style.border_alphas[index] = alpha;
                style.border_width = style.border_widths.into_iter().fold(0.0, f32::max);
            } else {
                bad = true;
            }
        }
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            let index = if key.contains("top") {
                0
            } else if key.contains("right") {
                1
            } else if key.contains("bottom") {
                2
            } else {
                3
            };
            if let Some(width) = border_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.border_widths[index] = width;
                style.border_width = style.border_widths.into_iter().fold(0.0, f32::max);
            } else {
                bad = true;
            }
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            let index = if key.contains("top") {
                0
            } else if key.contains("right") {
                1
            } else if key.contains("bottom") {
                2
            } else {
                3
            };
            if let Some((paint, alpha)) = color_with_alpha(v) {
                style.border_colors[index] = paint;
                style.border_alphas[index] = alpha;
            } else {
                bad = true;
            }
        }
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            let index = if key.contains("top") {
                0
            } else if key.contains("right") {
                1
            } else if key.contains("bottom") {
                2
            } else {
                3
            };
            if matches!(v, "none" | "solid" | "dashed" | "dotted") {
                style.border_styles[index] = v.into();
            } else {
                bad = true;
            }
        }
        "border-radius" => {
            if let Some(values) =
                box_values_ctx(v, style.font_size).filter(|v| v.iter().all(|n| *n >= 0.0))
            {
                style.border_radius = values;
            } else {
                bad = true;
            }
        }
        "width" => {
            if v == "auto" {
                style.width = None;
                style.width_percent = None;
            } else if let Some(n) = v
                .strip_suffix('%')
                .and_then(|p| p.trim().parse::<f32>().ok())
                .filter(|n| n.is_finite() && *n >= 0.0)
            {
                style.width_percent = Some(n / 100.0);
                style.width = None;
            } else if let Some(n) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.width = Some(n);
                style.width_percent = None;
            } else {
                bad = true
            }
        }
        "height" => {
            if v == "auto" {
                style.height = None;
                style.height_percent = None;
            } else if let Some(n) = v
                .strip_suffix('%')
                .and_then(|p| p.trim().parse::<f32>().ok())
                .filter(|n| n.is_finite() && *n >= 0.0)
            {
                style.height_percent = Some(n / 100.0);
                style.height = None;
            } else if let Some(n) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.height = Some(n);
                style.height_percent = None;
            } else {
                bad = true
            }
        }
        "min-width" => {
            if let Some(n) = v
                .strip_suffix('%')
                .and_then(|p| finite_f32(p.trim()))
                .filter(|n| *n >= 0.0)
            {
                style.min_width_percent = Some(n / 100.0);
                style.min_width = None;
            } else if let Some(n) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.min_width = Some(n);
                style.min_width_percent = None;
            } else {
                bad = true;
            }
        }
        "max-width" => {
            if v == "none" {
                style.max_width = None;
                style.max_width_percent = None;
            } else if let Some(n) = v
                .strip_suffix('%')
                .and_then(|p| finite_f32(p.trim()))
                .filter(|n| *n >= 0.0)
            {
                style.max_width_percent = Some(n / 100.0);
                style.max_width = None;
            } else if let Some(n) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.max_width = Some(n);
                style.max_width_percent = None;
            } else {
                bad = true;
            }
        }
        "min-height" => {
            if let Some(n) = v
                .strip_suffix('%')
                .and_then(|p| finite_f32(p.trim()))
                .filter(|n| *n >= 0.0)
            {
                style.min_height_percent = Some(n / 100.0);
                style.min_height = None;
            } else if let Some(n) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.min_height = Some(n);
                style.min_height_percent = None;
            } else {
                bad = true;
            }
        }
        "max-height" => {
            if v == "none" {
                style.max_height = None;
                style.max_height_percent = None;
            } else if let Some(n) = v
                .strip_suffix('%')
                .and_then(|p| finite_f32(p.trim()))
                .filter(|n| *n >= 0.0)
            {
                style.max_height_percent = Some(n / 100.0);
                style.max_height = None;
            } else if let Some(n) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.max_height = Some(n);
                style.max_height_percent = None;
            } else {
                bad = true;
            }
        }
        "aspect-ratio" => {
            style.aspect_ratio = v.parse::<f32>().ok().filter(|n| n.is_finite() && *n > 0.0);
            bad = style.aspect_ratio.is_none();
        }
        "break-before" | "page-break-before" if v == "page" || v == "always" => {
            style.break_before = true;
            style.break_before_side = None;
        }
        "break-before" | "page-break-before"
            if matches!(v, "left" | "right" | "recto" | "verso") =>
        {
            style.break_before = true;
            style.break_before_side = Some(
                if matches!(v, "left" | "verso") {
                    "left"
                } else {
                    "right"
                }
                .into(),
            );
        }
        "break-after" | "page-break-after" if v == "page" || v == "always" => {
            style.break_after = true;
            style.break_after_side = None;
        }
        "break-after" | "page-break-after" if matches!(v, "left" | "right" | "recto" | "verso") => {
            style.break_after = true;
            style.break_after_side = Some(
                if matches!(v, "left" | "verso") {
                    "left"
                } else {
                    "right"
                }
                .into(),
            );
        }
        "break-inside" | "page-break-inside" if v == "avoid" => style.break_inside_avoid = true,
        "table-layout" if matches!(v, "auto" | "fixed") => style.table_layout = v.into(),
        "border-collapse" if matches!(v, "collapse" | "separate") => {
            style.border_collapse = v.into()
        }
        "border-spacing" => {
            let values = v.split_whitespace().collect::<Vec<_>>();
            if matches!(values.len(), 1 | 2) {
                let horizontal = computed_length(values[0], style.font_size).filter(|n| *n >= 0.0);
                let vertical =
                    computed_length(values.get(1).copied().unwrap_or(values[0]), style.font_size)
                        .filter(|n| *n >= 0.0);
                if let (Some(horizontal), Some(vertical)) = (horizontal, vertical) {
                    style.border_spacing = [horizontal, vertical];
                } else {
                    bad = true;
                }
            } else {
                bad = true;
            }
        }
        "orphans" => {
            if let Some(value) = v
                .parse::<usize>()
                .ok()
                .filter(|value| (1..=1000).contains(value))
            {
                style.orphans = value;
            } else {
                bad = true;
            }
        }
        "widows" => {
            if let Some(value) = v
                .parse::<usize>()
                .ok()
                .filter(|value| (1..=1000).contains(value))
            {
                style.widows = value;
            } else {
                bad = true;
            }
        }
        "break-before" | "page-break-before" if v == "auto" => {
            style.break_before = false;
            style.break_before_side = None;
        }
        "break-after" | "page-break-after" if v == "auto" => {
            style.break_after = false;
            style.break_after_side = None;
        }
        "break-inside" | "page-break-inside" if v == "auto" => {
            style.break_inside_avoid = false;
        }
        _ => bad = true,
    }
    if bad && !matches!(key, "cursor" | "caret-color" | "user-select") {
        warnings.push(
            Diagnostic::new(
                "css-property",
                format!("unsupported CSS declaration: {key}: {v}"),
            )
            .property(key, v),
        );
    }
}
