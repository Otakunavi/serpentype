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

/// One resolved CSS Grid track function. Lengths are PDF points and
/// percentages are stored as fractions.
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrack {
    Fixed(f32),
    Percent(f32),
    Fr(f32),
    Auto,
    MinMax(Box<GridTrack>, Box<GridTrack>),
}

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
    pub bleed: f32,
    pub crop_marks: bool,
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

fn margin_box_font(value: &str) -> Option<(f32, Vec<String>)> {
    let value = value.trim();
    let size_end = value.find(char::is_whitespace)?;
    let size = length(&value[..size_end])?;
    let family_text = value[size_end..].trim();
    if family_text.is_empty() {
        return None;
    }
    let families = family_text
        .split(',')
        .map(|family| family.trim().trim_matches(['\'', '"']).trim().to_owned())
        .collect::<Vec<_>>();
    families
        .iter()
        .all(|family| !family.is_empty())
        .then_some((size, families))
}
impl Default for PageStyle {
    fn default() -> Self {
        Self {
            width: 595.28,
            height: 841.89,
            margin: [56.7; 4],
            rotation: 0,
            landscape_size_keyword: false,
            bleed: 0.0,
            crop_marks: false,
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
    pub running_name: Option<String>,
    pub counter_reset: Vec<(String, i32)>,
    pub counter_increment: Vec<(String, i32)>,
    pub string_set: Option<String>,
    pub inset: [Option<f32>; 4],
    pub inset_percent: [Option<f32>; 4],
    pub z_index: i32,
    /// CSS affine matrix in top-left coordinates: `[a, b, c, d, e, f]`.
    pub transform: [f32; 6],
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
    pub object_fit: String,
    pub object_position: [f32; 2],
    pub object_position_offset: [f32; 2],
    pub page_name: Option<String>,
    pub flex_direction: String,
    pub flex_wrap: String,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Option<f32>,
    pub flex_basis_percent: Option<f32>,
    pub align_items: String,
    pub align_self: String,
    pub align_content: String,
    pub justify_content: String,
    pub justify_items: String,
    pub justify_self: String,
    pub grid_columns: Option<usize>,
    pub grid_template_columns: Vec<GridTrack>,
    pub grid_template_rows: Vec<GridTrack>,
    pub grid_column_start: Option<usize>,
    pub grid_column_span: usize,
    pub grid_row_start: Option<usize>,
    pub grid_row_span: usize,
    pub row_gap: f32,
    pub column_gap: f32,
    pub column_count: usize,
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
            running_name: None,
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
            string_set: None,
            inset: [None; 4],
            inset_percent: [None; 4],
            z_index: 0,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
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
            object_fit: "fill".into(),
            object_position: [0.5, 0.5],
            object_position_offset: [0.0, 0.0],
            page_name: None,
            flex_direction: "row".into(),
            flex_wrap: "nowrap".into(),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: None,
            flex_basis_percent: None,
            align_items: "stretch".into(),
            align_self: "auto".into(),
            align_content: "stretch".into(),
            justify_content: "start".into(),
            justify_items: "stretch".into(),
            justify_self: "auto".into(),
            grid_columns: None,
            grid_template_columns: vec![],
            grid_template_rows: vec![],
            grid_column_start: None,
            grid_column_span: 1,
            grid_row_start: None,
            grid_row_span: 1,
            row_gap: 0.0,
            column_gap: 0.0,
            column_count: 1,
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
    page_rules: Vec<PageRule>,
    margin_rules: Vec<MarginRule>,
    pub rules: Vec<Rule>,
    pub faces: Vec<FontFace>,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
struct PageRule {
    page_name: Option<String>,
    pseudos: Vec<String>,
    declarations: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct MarginRule {
    page_name: Option<String>,
    pseudos: Vec<String>,
    box_name: String,
    declarations: Vec<(String, String)>,
}

impl Sheet {
    pub fn page_style_for(&self, name: Option<&str>, index: usize, blank: bool) -> PageStyle {
        type Priority = (bool, bool, usize, usize, usize);
        let mut winners: HashMap<String, (Priority, String)> = HashMap::new();
        for (rule_index, rule) in self.page_rules.iter().enumerate() {
            if rule
                .page_name
                .as_deref()
                .is_some_and(|page_name| Some(page_name) != name)
            {
                continue;
            }
            if !rule.pseudos.iter().all(|pseudo| match pseudo.as_str() {
                ":first" => index == 0,
                ":left" => (index + 1).is_multiple_of(2),
                ":right" => !(index + 1).is_multiple_of(2),
                ":blank" => blank,
                _ => false,
            }) {
                continue;
            }
            for (declaration_index, (property, raw)) in rule.declarations.iter().enumerate() {
                let priority = (
                    raw.trim().ends_with("!important"),
                    rule.page_name.is_some(),
                    rule.pseudos.len(),
                    rule_index,
                    declaration_index,
                );
                if winners
                    .get(property)
                    .is_none_or(|(old, _)| priority >= *old)
                {
                    winners.insert(property.clone(), (priority, raw.clone()));
                }
            }
        }
        let mut declarations = winners.into_iter().collect::<Vec<_>>();
        declarations.sort_by_key(|(_, (priority, _))| (priority.3, priority.4));
        let mut page = PageStyle::default();
        let mut ignored_warnings = Vec::new();
        parse_page_declarations(
            &mut page,
            declarations
                .into_iter()
                .map(|(property, (_, value))| (property, value))
                .collect(),
            &mut ignored_warnings,
        );
        page
    }

    pub fn resolved_page_styles(&self) -> Vec<PageStyle> {
        let mut names = vec![None];
        names.extend(
            self.page_rules
                .iter()
                .filter_map(|rule| rule.page_name.as_deref())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .map(Some),
        );
        names
            .into_iter()
            .flat_map(|name| {
                (0..2).flat_map(move |index| {
                    [false, true]
                        .into_iter()
                        .map(move |blank| self.page_style_for(name, index, blank))
                })
            })
            .collect()
    }

    /// Resolve page-margin declarations by importance, page-selector specificity,
    /// and source order. A named page outranks `:first` at equal importance.
    pub fn margin_boxes_for(
        &self,
        name: Option<&str>,
        index: usize,
        blank: bool,
    ) -> HashMap<String, MarginBox> {
        type Priority = (bool, bool, usize, usize, usize);
        let mut winners: HashMap<(String, String), (Priority, &str)> = HashMap::new();
        for (rule_index, rule) in self.margin_rules.iter().enumerate() {
            if !rule.pseudos.iter().all(|pseudo| match pseudo.as_str() {
                ":first" => index == 0,
                ":left" => (index + 1).is_multiple_of(2),
                ":right" => !(index + 1).is_multiple_of(2),
                ":blank" => blank,
                _ => false,
            }) {
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
                    "font" => margin_box_font(value).is_some(),
                    "font-size" => length(value).is_some(),
                    "color" => color(value).is_some(),
                    _ => false,
                };
                if !valid {
                    continue;
                }
                let priority = (
                    raw.trim().ends_with("!important"),
                    rule.page_name.is_some(),
                    rule.pseudos.len(),
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
                "font" => {
                    if let Some((size, family)) = margin_box_font(value) {
                        box_style.font_size = size;
                        box_style.family = family;
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
                    '-' | '_' | '.' | '#' | '[' | ']' | '=' | '"' | '\'' | ' ' | '>' | '*'
                )
        })
}
fn specificity(s: &str) -> u32 {
    (s.matches('#').count() as u32) * 100
        + ((s.matches('.').count() + s.matches('[').count()) as u32) * 10
        + u32::from(s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
}
fn valid_ident(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_' || ch == '-')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}
fn parse_counter_list(value: &str, default: i32) -> Option<Vec<(String, i32)>> {
    if value.trim() == "none" {
        return Some(Vec::new());
    }
    let parts: Vec<_> = value.split_whitespace().collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < parts.len() {
        let name = parts[index];
        if !valid_ident(name) {
            return None;
        }
        index += 1;
        let amount = match parts.get(index).and_then(|part| part.parse::<i32>().ok()) {
            Some(amount) => {
                index += 1;
                amount
            }
            None => default,
        };
        result.push((name.to_owned(), amount));
    }
    Some(result)
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
            "bleed" => {
                if let Some(value) = length(value).filter(|value| *value >= 0.0) {
                    page.bleed = value;
                } else {
                    unsupported(
                        warnings,
                        "css-value",
                        format!("invalid @page bleed: {value}"),
                    );
                }
            }
            "marks" if value == "crop" => page.crop_marks = true,
            "marks" if value == "none" => page.crop_marks = false,
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
            "font" => {
                if let Some((size, family)) = margin_box_font(value) {
                    box_style.font_size = size;
                    box_style.family = family;
                } else {
                    unsupported(
                        warnings,
                        "css-value",
                        format!("invalid margin box font: {value}"),
                    );
                }
            }
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
            let selector = selector.trim_start_matches("@page").trim();
            let parts: Vec<_> = selector.split(':').collect();
            let (page_name, pseudo_parts) = if selector.starts_with(':') {
                ("", &parts[1..])
            } else {
                (parts.first().copied().unwrap_or("").trim(), &parts[1..])
            };
            let pseudos = pseudo_parts
                .iter()
                .map(|pseudo| format!(":{}", pseudo.trim()))
                .collect::<Vec<_>>();
            let mut unique_pseudos = pseudos.clone();
            unique_pseudos.sort();
            unique_pseudos.dedup();
            let supported_pseudos = pseudos
                .iter()
                .all(|pseudo| matches!(pseudo.as_str(), ":first" | ":left" | ":right" | ":blank"))
                && unique_pseudos.len() == pseudos.len()
                && !(pseudos.contains(&":left".into()) && pseudos.contains(&":right".into()));
            if !supported_pseudos || page_name.contains(char::is_whitespace) || selector == ":" {
                unsupported(
                    &mut sheet.warnings,
                    "at-rule",
                    format!("unsupported @page selector: {selector}"),
                );
                continue;
            }
            let pseudo = (pseudos.len() == 1).then(|| pseudos[0].clone());
            let mut page = if !page_name.is_empty() {
                sheet
                    .named_pages
                    .get(page_name)
                    .cloned()
                    .unwrap_or_else(|| sheet.page.clone())
            } else if let Some(pseudo) = pseudo.as_deref() {
                sheet
                    .pseudo_pages
                    .get(pseudo)
                    .cloned()
                    .unwrap_or_else(|| sheet.page.clone())
            } else {
                sheet.page.clone()
            };
            let (nested, plain) = blocks(body)?;
            let page_declarations = declarations(&plain);
            parse_page_declarations(&mut page, page_declarations.clone(), &mut sheet.warnings);
            sheet.page_rules.push(PageRule {
                page_name: (!page_name.is_empty()).then(|| page_name.to_owned()),
                pseudos: pseudos.clone(),
                declarations: page_declarations,
            });
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
                        page_name: (!page_name.is_empty()).then(|| page_name.to_owned()),
                        pseudos: pseudos.clone(),
                        box_name: box_name.into(),
                        declarations: declarations(box_body),
                    });
                    if page_name.is_empty() && pseudos == [":first"] {
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
            if page_name.is_empty() && pseudos.is_empty() {
                sheet.page = page;
            } else if page_name.is_empty() && pseudos.len() == 1 {
                sheet.pseudo_pages.insert(pseudo.unwrap(), page);
            } else if !page_name.is_empty() && pseudos.is_empty() {
                sheet.named_pages.insert(page_name.into(), page);
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
        page_rules: Vec::new(),
        margin_rules: vec![],
        rules: vec![],
        faces: vec![],
        warnings: vec![],
    };
    parse_rules(&clean, &mut sheet)?;
    if strict && !sheet.warnings.is_empty() {
        let warning = &sheet.warnings[0];
        return Err(Error(format!(
            "strict mode: {}: {}",
            warning.code, warning.message
        )));
    }
    Ok(sheet)
}

pub fn selector_matches(selector: &str, tag: &str, id: Option<&str>, classes: &str) -> bool {
    let mut remaining = selector;
    let tag_end = remaining.find(['.', '#']).unwrap_or(remaining.len());
    let wanted_tag = &remaining[..tag_end];
    if !wanted_tag.is_empty() && wanted_tag != "*" && wanted_tag != tag {
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

fn split_top_level(value: &str, separator: char) -> Vec<&str> {
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut out = Vec::new();
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if ch == separator && depth == 0 => {
                out.push(value[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    out.push(value[start..].trim());
    out
}

fn grid_track(value: &str, font_size: f32) -> Option<GridTrack> {
    let value = value.trim();
    if value == "auto" {
        return Some(GridTrack::Auto);
    }
    if let Some(number) = value.strip_suffix("fr") {
        return finite_f32(number.trim())
            .filter(|number| *number > 0.0)
            .map(GridTrack::Fr);
    }
    if let Some(number) = value.strip_suffix('%') {
        return finite_f32(number.trim())
            .filter(|number| *number >= 0.0)
            .map(|number| GridTrack::Percent(number / 100.0));
    }
    if let Some(body) = value
        .strip_prefix("minmax(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let values = split_top_level(body, ',');
        if values.len() == 2 {
            return Some(GridTrack::MinMax(
                Box::new(grid_track(values[0], font_size)?),
                Box::new(grid_track(values[1], font_size)?),
            ));
        }
        return None;
    }
    computed_length(value, font_size)
        .filter(|number| *number >= 0.0)
        .map(GridTrack::Fixed)
}

fn grid_tracks(value: &str, font_size: f32) -> Option<Vec<GridTrack>> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => depth = depth.checked_sub(1)?,
            _ if ch.is_whitespace() && depth == 0 => {
                if let Some(begin) = start.take() {
                    tokens.push(value[begin..index].trim());
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(begin) = start {
        tokens.push(value[begin..].trim());
    }
    let mut out = Vec::new();
    for token in tokens {
        if let Some(body) = token
            .strip_prefix("repeat(")
            .and_then(|value| value.strip_suffix(')'))
        {
            let values = split_top_level(body, ',');
            if values.len() != 2 {
                return None;
            }
            let count = values[0].parse::<usize>().ok().filter(|count| *count > 0)?;
            let repeated = grid_tracks(values[1], font_size)?;
            if repeated.is_empty() || count.saturating_mul(repeated.len()) > 256 {
                return None;
            }
            for _ in 0..count {
                out.extend(repeated.iter().cloned());
            }
        } else {
            out.push(grid_track(token, font_size)?);
        }
    }
    (!out.is_empty()).then_some(out)
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

fn transform_matrix(value: &str, font_size: f32) -> Option<[f32; 6]> {
    if value == "none" {
        return Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
    let mut rest = value.trim();
    let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let name = rest[..open].trim();
        let mut depth = 1usize;
        let mut close = None;
        for (offset, ch) in rest[open + 1..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + 1 + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        let args: Vec<_> = rest[open + 1..close]
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|value| !value.is_empty())
            .collect();
        let number = |index: usize| args.get(index).and_then(|raw| finite_f32(raw));
        let distance = |index: usize| {
            args.get(index)
                .and_then(|raw| computed_length(raw, font_size))
        };
        let operation = match name {
            "translate" if (1..=2).contains(&args.len()) => [
                1.0,
                0.0,
                0.0,
                1.0,
                distance(0)?,
                if args.len() == 2 { distance(1)? } else { 0.0 },
            ],
            "translateX" if args.len() == 1 => [1.0, 0.0, 0.0, 1.0, distance(0)?, 0.0],
            "translateY" if args.len() == 1 => [1.0, 0.0, 0.0, 1.0, 0.0, distance(0)?],
            "scale" if (1..=2).contains(&args.len()) => {
                let x = number(0)?;
                [
                    x,
                    0.0,
                    0.0,
                    if args.len() == 2 { number(1)? } else { x },
                    0.0,
                    0.0,
                ]
            }
            "scaleX" if args.len() == 1 => [number(0)?, 0.0, 0.0, 1.0, 0.0, 0.0],
            "scaleY" if args.len() == 1 => [1.0, 0.0, 0.0, number(0)?, 0.0, 0.0],
            "rotate" if args.len() == 1 => {
                let angle = if let Some(degrees) = args[0].strip_suffix("deg") {
                    finite_f32(degrees)?.to_radians()
                } else if let Some(turns) = args[0].strip_suffix("turn") {
                    finite_f32(turns)? * std::f32::consts::TAU
                } else {
                    let radians = args[0].strip_suffix("rad")?;
                    finite_f32(radians)?
                };
                let (sin, cos) = angle.sin_cos();
                [cos, sin, -sin, cos, 0.0, 0.0]
            }
            _ => return None,
        };
        matrix = affine_multiply(matrix, operation);
        rest = rest[close + 1..].trim_start();
    }
    Some(matrix)
}

fn grid_placement(value: &str) -> Option<(Option<usize>, usize)> {
    let parts = split_top_level(value, '/');
    let first = parts.first()?.trim();
    if let Some(raw) = first.strip_prefix("span ") {
        if parts.len() != 1 {
            return None;
        }
        return Some((
            None,
            raw.trim()
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0)?,
        ));
    }
    let start = match first {
        "auto" => None,
        raw => Some(raw.parse::<usize>().ok().filter(|value| *value > 0)?),
    };
    let span = if let Some(end) = parts.get(1) {
        let raw = end.trim();
        if let Some(raw) = raw.strip_prefix("span ") {
            raw.trim()
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0)?
        } else if let (Some(start), Ok(end)) = (start, raw.parse::<usize>()) {
            end.checked_sub(start).filter(|value| *value > 0)?
        } else {
            return None;
        }
    } else {
        1
    };
    Some((start, span))
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
        "position" if matches!(v, "static" | "relative" | "absolute" | "fixed") => {
            style.position = v.into();
            style.running_name = None;
        }
        "position" if v.starts_with("running(") && v.ends_with(')') => {
            let name = v[8..v.len() - 1].trim();
            if valid_ident(name) {
                style.position = "running".into();
                style.running_name = Some(name.into());
            } else {
                bad = true;
            }
        }
        "counter-reset" | "counter-increment" => {
            if let Some(parsed) = parse_counter_list(v, if key == "counter-reset" { 0 } else { 1 })
            {
                if key == "counter-reset" {
                    style.counter_reset = parsed;
                } else {
                    style.counter_increment = parsed;
                }
            } else {
                bad = true;
            }
        }
        "string-set" => {
            if let Some((name, expression)) = v.split_once(char::is_whitespace) {
                if valid_ident(name) && expression.trim().starts_with("content(") {
                    style.string_set = Some(name.into());
                } else {
                    bad = true;
                }
            } else {
                bad = true;
            }
        }
        "z-index" if v == "auto" => style.z_index = 0,
        "z-index" => {
            if let Ok(value) = v.parse::<i32>() {
                style.z_index = value;
            } else {
                bad = true;
            }
        }
        "transform" => {
            if let Some(matrix) = transform_matrix(v, style.font_size) {
                style.transform = matrix;
            } else {
                bad = true;
            }
        }
        // Transforms are centered by default in the PDF painter, so the
        // explicit CSS default does not alter geometry.
        "transform-origin" if matches!(v, "center" | "50% 50%") => {}
        // These identity declarations have no effect on the vector display
        // list. Non-identity filters and origins continue to be diagnosed.
        "filter" if v == "none" => {}
        "filter" if v.starts_with("blur(") && v.ends_with(')') => {
            let radius = &v[5..v.len() - 1];
            if computed_length(radius, style.font_size) != Some(0.0) {
                bad = true;
            }
        }
        "outline" if v == "none" => {}
        "top" | "right" | "bottom" | "left" => {
            let idx = match key {
                "top" => 0,
                "right" => 1,
                "bottom" => 2,
                _ => 3,
            };
            if v == "auto" {
                style.inset[idx] = None;
                style.inset_percent[idx] = None;
            } else if let Some(percent) = v.strip_suffix('%').and_then(finite_f32) {
                style.inset[idx] = None;
                style.inset_percent[idx] = Some(percent / 100.0);
            } else if let Some(n) = computed_length(v, style.font_size) {
                style.inset[idx] = Some(n);
                style.inset_percent[idx] = None;
            } else {
                bad = true;
            }
        }
        "page" if v == "auto" => style.page_name = None,
        "page" if !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
            style.page_name = Some(v.into())
        }
        "flex-direction" if matches!(v, "column" | "column-reverse" | "row" | "row-reverse") => {
            style.flex_direction = v.into()
        }
        "flex-wrap" if matches!(v, "nowrap" | "wrap" | "wrap-reverse") => {
            style.flex_wrap = v.into()
        }
        "flex-grow" | "flex-shrink" => {
            if let Some(number) = finite_f32(v).filter(|number| *number >= 0.0) {
                if key == "flex-grow" {
                    style.flex_grow = number;
                } else {
                    style.flex_shrink = number;
                }
            } else {
                bad = true;
            }
        }
        "flex-basis" => {
            if v == "auto" {
                style.flex_basis = None;
                style.flex_basis_percent = None;
            } else if let Some(percent) = v.strip_suffix('%').and_then(finite_f32) {
                if percent >= 0.0 {
                    style.flex_basis = None;
                    style.flex_basis_percent = Some(percent / 100.0);
                } else {
                    bad = true;
                }
            } else if let Some(length) = computed_length(v, style.font_size).filter(|n| *n >= 0.0) {
                style.flex_basis = Some(length);
                style.flex_basis_percent = None;
            } else {
                bad = true;
            }
        }
        "flex" => {
            let values: Vec<_> = v.split_whitespace().collect();
            if values.len() == 1 && v == "none" {
                style.flex_grow = 0.0;
                style.flex_shrink = 0.0;
                style.flex_basis = None;
                style.flex_basis_percent = None;
            } else if (1..=3).contains(&values.len()) {
                let grow = finite_f32(values[0]).filter(|n| *n >= 0.0);
                let shrink = values
                    .get(1)
                    .and_then(|raw| finite_f32(raw))
                    .filter(|n| *n >= 0.0);
                if let Some(grow) = grow {
                    style.flex_grow = grow;
                    style.flex_shrink = shrink.unwrap_or(1.0);
                    if let Some(basis) = values.get(2) {
                        if let Some(percent) = basis.strip_suffix('%').and_then(finite_f32) {
                            style.flex_basis_percent = Some(percent / 100.0);
                            style.flex_basis = None;
                        } else if let Some(length) = computed_length(basis, style.font_size) {
                            style.flex_basis = Some(length);
                            style.flex_basis_percent = None;
                        } else {
                            bad = true;
                        }
                    }
                } else {
                    bad = true;
                }
            } else {
                bad = true;
            }
        }
        "align-items" if matches!(v, "center" | "start" | "end" | "stretch") => {
            style.align_items = v.into()
        }
        "align-self" if matches!(v, "auto" | "center" | "start" | "end" | "stretch") => {
            style.align_self = v.into()
        }
        "align-content"
            if matches!(
                v,
                "start" | "end" | "center" | "stretch" | "space-between" | "space-around"
            ) =>
        {
            style.align_content = v.into()
        }
        "justify-content"
            if matches!(
                v,
                "center" | "start" | "end" | "space-between" | "space-around"
            ) =>
        {
            style.justify_content = v.into()
        }
        "justify-items" if matches!(v, "center" | "start" | "end" | "stretch") => {
            style.justify_items = v.into()
        }
        "justify-self" if matches!(v, "auto" | "center" | "start" | "end" | "stretch") => {
            style.justify_self = v.into()
        }
        "grid-template-columns" | "grid-template-rows" => {
            if let Some(tracks) = grid_tracks(v, style.font_size) {
                if key == "grid-template-columns" {
                    style.grid_columns = Some(tracks.len());
                    style.grid_template_columns = tracks;
                } else {
                    style.grid_template_rows = tracks;
                }
            } else {
                bad = true;
            }
        }
        "grid-column" | "grid-row" => {
            if let Some((start, span)) = grid_placement(v) {
                if key == "grid-column" {
                    style.grid_column_start = start;
                    style.grid_column_span = span;
                } else {
                    style.grid_row_start = start;
                    style.grid_row_span = span;
                }
            } else {
                bad = true;
            }
        }
        "grid-column-start" | "grid-row-start" => {
            let parsed = if v == "auto" {
                Some(None)
            } else {
                v.parse::<usize>().ok().filter(|value| *value > 0).map(Some)
            };
            if let Some(start) = parsed {
                if key == "grid-column-start" {
                    style.grid_column_start = start;
                } else {
                    style.grid_row_start = start;
                }
            } else {
                bad = true;
            }
        }
        "grid-column-end" | "grid-row-end" => {
            if let Some(raw) = v.strip_prefix("span ") {
                if let Ok(span) = raw.trim().parse::<usize>() {
                    if span == 0 {
                        bad = true;
                    } else if key == "grid-column-end" {
                        style.grid_column_span = span;
                    } else {
                        style.grid_row_span = span;
                    }
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
        "column-count" => {
            if let Ok(count) = v.parse::<usize>() {
                if (1..=16).contains(&count) {
                    style.column_count = count;
                } else {
                    bad = true;
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
        "object-fit" if matches!(v, "fill" | "contain" | "cover" | "none" | "scale-down") => {
            style.object_fit = v.into();
        }
        "object-position" => {
            let position_component = |value: &str, horizontal: bool| -> Option<(f32, f32)> {
                match value {
                    "left" if horizontal => Some((0.0, 0.0)),
                    "right" if horizontal => Some((1.0, 0.0)),
                    "top" if !horizontal => Some((0.0, 0.0)),
                    "bottom" if !horizontal => Some((1.0, 0.0)),
                    "center" => Some((0.5, 0.0)),
                    _ => value
                        .strip_suffix('%')
                        .and_then(finite_f32)
                        .map(|value| (value / 100.0, 0.0))
                        .or_else(|| {
                            computed_length(value, style.font_size).map(|offset| (0.0, offset))
                        }),
                }
            };
            let values: Vec<_> = v.split_whitespace().collect();
            let parsed = match values.as_slice() {
                [one] => position_component(one, true)
                    .map(|x| [x, (0.5, 0.0)])
                    .or_else(|| position_component(one, false).map(|y| [(0.5, 0.0), y])),
                [first, second] => position_component(first, true)
                    .zip(position_component(second, false))
                    .map(|(x, y)| [x, y])
                    .or_else(|| {
                        position_component(first, false)
                            .zip(position_component(second, true))
                            .map(|(y, x)| [x, y])
                    }),
                _ => None,
            };
            if let Some(position) = parsed {
                style.object_position = position.map(|component| component.0);
                style.object_position_offset = position.map(|component| component.1);
            } else {
                bad = true;
            }
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
