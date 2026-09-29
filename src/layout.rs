//! Page-aware flow layout. The output display list is immutable input to PDF export.
use crate::css::{self, Color, MarginBox, PageStyle, Sheet, Style};
use crate::font::{FontData, FontRegistry};
use crate::html::{self, Node};
use crate::{Diagnostic, Error, Result};
use image::GenericImageView;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const EPS: f32 = 0.02;
/// A fixed RGB raster loaded at layout time.
#[derive(Debug)]
pub struct ImageData {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub digest: [u8; 32],
}
#[derive(Default)]
struct ImageCache {
    entries: HashMap<[u8; 32], Arc<ImageData>>,
    order: VecDeque<[u8; 32]>,
    bytes: usize,
}
const IMAGE_CACHE_LIMIT: usize = 32 * 1024 * 1024;
/// Positioned PDF primitives in top-left coordinates, measured in points.
#[derive(Clone, Debug)]
pub enum Item {
    Text {
        ch: char,
        glyph: u16,
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
}
fn shift_item(item: &mut Item, shift: f32) {
    match item {
        Item::Text { y, .. } | Item::Rect { y, .. } | Item::Image { y, .. } => *y += shift,
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
}
impl PreparedDocument {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    pub fn to_pdf(&self) -> Result<Vec<u8>> {
        crate::pdf::export(self)
    }
}
/// Reusable renderer. Each call uses an isolated registration snapshot.
#[derive(Clone)]
pub struct Renderer {
    pub fonts: FontRegistry,
    pub base_dir: PathBuf,
    pub strict: bool,
    images: Arc<Mutex<ImageCache>>,
}
impl Renderer {
    /// Create a renderer with an explicit font registry and local-resource base directory.
    pub fn new(fonts: FontRegistry, base_dir: PathBuf, strict: bool) -> Self {
        Self {
            fonts,
            base_dir,
            strict,
            images: Arc::new(Mutex::new(ImageCache::default())),
        }
    }
    /// Parse, measure and paginate a document without creating PDF bytes.
    pub fn layout(&self, html: &str, css_text: &str) -> Result<PreparedDocument> {
        let all_css = format!("{}\n{}", html::embedded_css(html), css_text);
        let sheet = css::parse(&all_css, self.strict)?;
        if sheet.page.width <= sheet.page.margin[1] + sheet.page.margin[3]
            || sheet.page.height <= sheet.page.margin[0] + sheet.page.margin[2]
            || sheet.page.margin.iter().any(|v| *v < 0.0)
        {
            return Err(Error(
                "@page margins leave no positive content rectangle".into(),
            ));
        }
        let fonts = self.fonts.snapshot();
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
            fonts.register_file(
                &self.base_dir.join(path).to_string_lossy(),
                &face.family,
                face.weight,
                &face.style,
            )?;
        }
        let (root, warnings) = html::parse(html, &sheet, self.strict)?;
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
        let mut flow = Flow::new(
            sheet.clone(),
            fonts,
            self.base_dir.clone(),
            self.images.clone(),
            warnings,
        );
        for child in &root.children {
            flow.node(child)?;
        }
        flow.render_margin_boxes()?;
        Ok(PreparedDocument {
            pages: flow.pages,
            page_style: sheet.page,
            warnings: flow.warnings,
        })
    }
}

#[derive(Clone)]
struct Glyph {
    ch: char,
    id: u16,
    font: Arc<FontData>,
    advance: f32,
    size: f32,
    color: Color,
}
#[derive(Clone, Default)]
struct Line {
    glyphs: Vec<Glyph>,
    width: f32,
    height: f32,
}
fn push_line(lines: &mut Vec<Line>, mut glyphs: Vec<Glyph>, default_height: f32) {
    while glyphs.last().is_some_and(|g| g.ch == ' ') {
        glyphs.pop();
    }
    let width = glyphs.iter().map(|g| g.advance).sum();
    let height = glyphs
        .iter()
        .map(|g| g.size * 1.2)
        .fold(default_height, f32::max);
    lines.push(Line {
        glyphs,
        width,
        height,
    });
}
fn inline_chars(node: &Node, out: &mut Vec<(char, Style)>) {
    if node.tag == "br" {
        out.push(('\n', node.style.clone()));
        return;
    }
    for ch in node.text.chars() {
        out.push((ch, node.style.clone()));
    }
    for child in &node.children {
        inline_chars(child, out);
    }
}
fn lines_for(node: &Node, width: f32, fonts: &FontRegistry) -> Result<Vec<Line>> {
    let mut chars = Vec::new();
    inline_chars(node, &mut chars);
    let mut lines = Vec::new();
    let mut current = Vec::<Glyph>::new();
    let mut current_width = 0.0;
    let default_height = node.style.line_height.max(node.style.font_size);
    let mut previous_space = false;
    for (raw, style) in chars {
        if raw == '\n' {
            push_line(&mut lines, std::mem::take(&mut current), default_height);
            current_width = 0.0;
            previous_space = false;
            continue;
        }
        let ch = if raw.is_whitespace() { ' ' } else { raw };
        if ch == ' ' && (previous_space || current.is_empty()) {
            continue;
        }
        previous_space = ch == ' ';
        let font = fonts.resolve(&style.family, style.weight, &style.font_style, ch)?;
        let (id, units) = font.glyph(ch)?;
        let advance = units as f32 * style.font_size / font.units_per_em as f32;
        let glyph = Glyph {
            ch,
            id,
            font,
            advance,
            size: style.font_size,
            color: style.color,
        };
        if current_width + advance > width + EPS && !current.is_empty() {
            // Prefer the last word boundary; an overwide word falls back to glyph breaks.
            if let Some(last_space) = current.iter().rposition(|g| g.ch == ' ') {
                let trailing = current.split_off(last_space + 1);
                current.pop();
                push_line(&mut lines, std::mem::take(&mut current), default_height);
                current = trailing;
                current_width = current.iter().map(|g| g.advance).sum();
            } else {
                push_line(&mut lines, std::mem::take(&mut current), default_height);
                current_width = 0.0;
            }
        }
        if ch != ' ' || !current.is_empty() {
            current_width += advance;
            current.push(glyph);
        }
    }
    if !current.is_empty() || lines.is_empty() {
        push_line(&mut lines, current, default_height);
    }
    Ok(lines)
}

struct Flow {
    pages: Vec<Page>,
    y: f32,
    page: PageStyle,
    sheet: Sheet,
    page_name: Option<String>,
    fonts: FontRegistry,
    base_dir: PathBuf,
    images: Arc<Mutex<ImageCache>>,
    warnings: Vec<Diagnostic>,
    pending_break: bool,
    frame: Option<(f32, f32)>,
    center_children: bool,
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
        let lines = lines_for(&node, w, &self.fonts)?;
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
                    font: glyph.font,
                    x: pen,
                    y: cursor + glyph.size,
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
    ) -> Self {
        let mut page = sheet.page.clone();
        if page.landscape_size_keyword {
            page.rotation = 0;
        }
        page.boxes = sheet.margin_boxes_for(None, true);
        Self {
            pages: vec![Page {
                items: vec![],
                style: page.clone(),
                name: None,
            }],
            y: page.margin[0],
            page,
            sheet,
            page_name: None,
            fonts,
            base_dir,
            images,
            warnings,
            pending_break: false,
            frame: None,
            center_children: false,
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
        self.pages.last().is_some_and(|p| !p.items.is_empty())
    }
    fn new_page(&mut self) {
        self.page = self.style_for(self.page_name.as_deref(), self.pages.len());
        self.pages.push(Page {
            items: vec![],
            style: self.page.clone(),
            name: self.page_name.clone(),
        });
        self.y = self.page.margin[0];
    }
    fn style_for(&self, name: Option<&str>, index: usize) -> PageStyle {
        let mut style = name
            .and_then(|n| self.sheet.named_pages.get(n))
            .cloned()
            .unwrap_or_else(|| self.sheet.page.clone());
        // `size: landscape` already selects a landscape sheet. In this
        // compatibility case, a second 90-degree PDF page rotation would
        // display the landscape content sideways in common PDF viewers.
        if style.landscape_size_keyword {
            style.rotation = 0;
        }
        style.boxes = self.sheet.margin_boxes_for(name, index == 0);
        style
    }
    fn switch_page(&mut self, name: Option<&str>) {
        if self.page_name.as_deref() == name {
            return;
        }
        self.page_name = name.map(str::to_owned);
        if self.page_has_content() {
            self.new_page();
        } else {
            self.page = self.style_for(name, self.pages.len() - 1);
            let page = self.pages.last_mut().expect("initial page");
            page.style = self.page.clone();
            page.name = self.page_name.clone();
            self.y = self.page.margin[0];
        }
    }
    fn begin(&mut self, style: &Style) {
        self.switch_page(style.page_name.as_deref());
        if (self.pending_break || style.break_before) && self.page_has_content() {
            self.new_page();
        }
        self.pending_break = false;
    }
    fn finish(&mut self, style: &Style) {
        if style.break_after {
            self.pending_break = true;
        }
    }
    fn item(&mut self, item: Item) {
        if let Some(page) = self.pages.last_mut() {
            page.items.push(item);
        }
    }
    fn node(&mut self, node: &Node) -> Result<()> {
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
        self.begin(&node.style);
        let mut group = Vec::new();
        for child in &node.children {
            if matches!(
                child.tag.as_str(),
                "#text" | "span" | "strong" | "b" | "em" | "i" | "br"
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
        self.begin(&node.style);
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
        for child in &node.children {
            self.node(child)?;
        }
        self.frame = old_frame;
        self.center_children = old_center;
        if let Some(height) = node.style.height {
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
        self.begin(&node.style);
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
        for row in children.chunks(columns) {
            if self.limit() - self.y < 40.0 && self.page_has_content() {
                self.new_page();
            }
            let row_y = self.y;
            let row_page = self.pages.len();
            let mut cell_ranges = Vec::new();
            let mut row_end = row_y;
            for (col, child) in row.iter().enumerate() {
                self.y = row_y;
                self.frame = Some((
                    x + col as f32 * width / columns as f32,
                    width / columns as f32,
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
        }
        self.frame = old_frame;
        self.y += node.style.padding[2] + node.style.margin[2];
        self.finish(&node.style);
        Ok(())
    }
    fn paint_box(&mut self, x: f32, y: f32, w: f32, h: f32, style: &Style) {
        if style.background.is_some() || style.border_width > 0.0 {
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
    fn draw_line(&mut self, line: &Line, x: f32, y: f32, width: f32, align: &str) {
        let offset = match align {
            "center" => (width - line.width) / 2.0,
            "right" => width - line.width,
            _ => 0.0,
        };
        let mut pen = x + offset.max(0.0);
        for glyph in &line.glyphs {
            self.item(Item::Text {
                ch: glyph.ch,
                glyph: glyph.id,
                font: glyph.font.clone(),
                x: pen,
                y: y + glyph.size,
                size: glyph.size,
                color: glyph.color,
            });
            pen += glyph.advance;
        }
    }
    fn paragraph(&mut self, node: &Node) -> Result<()> {
        self.begin(&node.style);
        let s = &node.style;
        let x = self.content_x() + s.margin[3];
        let w = s
            .width
            .unwrap_or(self.content_width() - s.margin[1] - s.margin[3])
            .min(self.content_width());
        let inner = w - s.padding[1] - s.padding[3] - 2.0 * s.border_width;
        if inner <= 0.0 {
            return Err(Error("paragraph has no usable width".into()));
        }
        let lines = lines_for(node, inner, &self.fonts)?;
        let body_height: f32 = lines.iter().map(|l| l.height).sum();
        let box_height = body_height + s.padding[0] + s.padding[2] + 2.0 * s.border_width;
        let full_needed = s.margin[0] + box_height + s.margin[2];
        if (s.break_inside_avoid || lines.len() == 1)
            && full_needed <= self.full_height() + EPS
            && self.y + full_needed > self.limit() + EPS
            && self.page_has_content()
        {
            self.new_page();
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
                    self.new_page();
                    continue;
                }
                return Err(Error(
                    "text line and padding exceed page content height".into(),
                ));
            }
            self.paint_box(x, start, w, used, s);
            let mut line_y = start + s.border_width + s.padding[0];
            for line in &lines[offset..end] {
                self.draw_line(
                    line,
                    x + s.border_width + s.padding[3],
                    line_y,
                    inner,
                    &s.text_align,
                );
                line_y += line.height;
            }
            self.y += used;
            offset = end;
            if offset < lines.len() {
                self.new_page();
            }
        }
        self.y += s.margin[2];
        self.finish(s);
        Ok(())
    }
    fn image(&mut self, node: &Node) -> Result<()> {
        self.begin(&node.style);
        let src = node
            .attr("src")
            .ok_or_else(|| Error("img needs src".into()))?;
        if src.contains("://") || src.starts_with("data:") {
            return Err(Error("only local image files are supported".into()));
        }
        let path = self.base_dir.join(Path::new(src));
        let bytes =
            std::fs::read(&path).map_err(|e| Error(format!("image {}: {e}", path.display())))?;
        let digest = Sha256::digest(&bytes).into();
        let hit = self
            .images
            .lock()
            .map_err(|e| Error(e.to_string()))?
            .entries
            .get(&digest)
            .cloned();
        let data = if let Some(hit) = hit {
            hit
        } else {
            let image =
                image::load_from_memory(&bytes).map_err(|e| Error(format!("image decode: {e}")))?;
            let (iw, ih) = image.dimensions();
            // PDF RGB has no alpha channel; composite transparent PNG pixels on white.
            let rgba = image.to_rgba8();
            let mut rgb = Vec::with_capacity((iw as usize) * (ih as usize) * 3);
            for pixel in rgba.pixels() {
                let alpha = u32::from(pixel[3]);
                for channel in 0..3 {
                    rgb.push(
                        ((u32::from(pixel[channel]) * alpha + 255 * (255 - alpha)) / 255) as u8,
                    );
                }
            }
            let data = Arc::new(ImageData {
                rgb,
                width: iw,
                height: ih,
                digest,
            });
            let mut cache = self.images.lock().map_err(|e| Error(e.to_string()))?;
            let size = data.rgb.len();
            if size <= IMAGE_CACHE_LIMIT {
                while cache.bytes + size > IMAGE_CACHE_LIMIT {
                    let Some(old) = cache.order.pop_front() else {
                        break;
                    };
                    if let Some(item) = cache.entries.remove(&old) {
                        cache.bytes -= item.rgb.len();
                    }
                }
                cache.bytes += size;
                cache.order.push_back(digest);
                cache.entries.insert(digest, data.clone());
            }
            data
        };
        let (iw, ih) = (data.width, data.height);
        let mut width = node
            .style
            .width
            .unwrap_or(iw as f32 * 0.75)
            .min(self.content_width());
        if let Some(percent) = node.style.max_width_percent {
            width = width.min(self.content_width() * percent);
        }
        let mut height = node
            .style
            .height
            .unwrap_or_else(|| width / node.style.aspect_ratio.unwrap_or(iw as f32 / ih as f32));
        if let Some(percent) = node.style.max_height_percent {
            let max_height = self.full_height() * percent;
            if height > max_height {
                let scale = max_height / height;
                height = max_height;
                width *= scale;
            }
        }
        if height > self.full_height() + EPS {
            return Err(Error("image exceeds page content height".into()));
        }
        if self.y + height > self.limit() + EPS && self.page_has_content() {
            self.new_page();
        }
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
}
#[derive(Clone)]
struct Row {
    cells: Vec<Cell>,
    line_count: usize,
    step: f32,
    pad: f32,
}
impl Flow {
    fn rows(node: &Node, in_head: bool, out: &mut Vec<(Node, bool)>) {
        for child in &node.children {
            if child.tag == "tr" {
                out.push((child.clone(), in_head));
            } else if matches!(child.tag.as_str(), "thead" | "tbody" | "tfoot") {
                Self::rows(child, in_head || child.tag == "thead", out);
            }
        }
    }
    fn table(&mut self, table: &Node) -> Result<()> {
        self.begin(&table.style);
        let mut row_nodes = Vec::new();
        Self::rows(table, false, &mut row_nodes);
        if row_nodes.is_empty() {
            self.finish(&table.style);
            return Ok(());
        }
        let columns = row_nodes
            .iter()
            .map(|(r, _)| {
                r.children
                    .iter()
                    .filter(|c| matches!(c.tag.as_str(), "td" | "th"))
                    .count()
            })
            .max()
            .unwrap_or(0);
        if columns == 0 {
            self.finish(&table.style);
            return Ok(());
        }
        let width = table
            .style
            .width
            .unwrap_or(self.content_width())
            .min(self.content_width());
        let widths = self.column_widths(&row_nodes, columns, width)?;
        let mut heads = Vec::new();
        let mut bodies = Vec::new();
        for (row, is_head) in row_nodes {
            let layout = self.layout_row(&row, &widths)?;
            if is_head && bodies.is_empty() {
                heads.push(layout)
            } else {
                bodies.push(layout)
            }
        }
        let header_height: f32 = heads
            .iter()
            .map(|r| r.line_count as f32 * r.step + r.pad)
            .sum();
        if header_height >= self.full_height() - EPS {
            return Err(Error(
                "repeating table header fills page content area".into(),
            ));
        }
        let total_height = header_height
            + bodies
                .iter()
                .map(|r| r.line_count as f32 * r.step + r.pad)
                .sum::<f32>();
        if table.style.break_inside_avoid
            && total_height <= self.full_height() + EPS
            && self.y + total_height > self.limit() + EPS
            && self.page_has_content()
        {
            self.new_page();
        }
        let mut header_drawn = false;
        for row in &bodies {
            if !header_drawn {
                self.ensure_header(&heads, &widths, header_height)?;
                header_drawn = true;
            }
            let mut offset = 0;
            while offset < row.line_count {
                let remaining = row.line_count - offset;
                let row_height = remaining as f32 * row.step + row.pad;
                if row_height <= self.full_height() - header_height + EPS
                    && self.y + row_height > self.limit() + EPS
                    && self.page_has_content()
                {
                    self.new_page();
                    self.ensure_header(&heads, &widths, header_height)?;
                }
                let available = self.limit() - self.y - row.pad;
                let take = ((available + EPS) / row.step).floor().max(0.0) as usize;
                if take == 0 {
                    if self.y > self.page.margin[0] + header_height + EPS {
                        self.new_page();
                        self.ensure_header(&heads, &widths, header_height)?;
                        continue;
                    }
                    return Err(Error(
                        "table row cannot place one text line after header".into(),
                    ));
                }
                let take = take.min(remaining);
                self.draw_row(row, &widths, offset, take);
                offset += take;
                if offset < row.line_count {
                    self.new_page();
                    self.ensure_header(&heads, &widths, header_height)?;
                }
            }
        }
        if bodies.is_empty() {
            self.ensure_header(&heads, &widths, header_height)?;
        }
        self.finish(&table.style);
        Ok(())
    }
    // Explicit cell widths are honored first; remaining space follows content minimums.
    fn column_widths(&self, rows: &[(Node, bool)], columns: usize, total: f32) -> Result<Vec<f32>> {
        let mut desired = vec![20.0f32; columns];
        let mut explicit = vec![false; columns];
        for (row, _) in rows {
            for (i, cell) in row
                .children
                .iter()
                .filter(|c| matches!(c.tag.as_str(), "td" | "th"))
                .enumerate()
            {
                if let Some(w) = cell.style.width {
                    desired[i] = desired[i].max(w);
                    explicit[i] = true;
                } else if !explicit[i] {
                    let longest = cell
                        .plain_text()
                        .split_whitespace()
                        .map(|w| w.chars().count())
                        .max()
                        .unwrap_or(0)
                        .min(40);
                    desired[i] = desired[i].max(
                        longest as f32 * cell.style.font_size * 0.55
                            + cell.style.padding[1]
                            + cell.style.padding[3]
                            + 2.0 * cell.style.border_width,
                    );
                }
            }
        }
        let sum: f32 = desired.iter().sum();
        if sum <= 0.0 {
            return Err(Error("table width is zero".into()));
        }
        Ok(desired.into_iter().map(|w| w * total / sum).collect())
    }
    fn layout_row(&self, row: &Node, widths: &[f32]) -> Result<Row> {
        let mut cells = Vec::new();
        let mut count = 1;
        let mut step = 0.0f32;
        let mut pad = 0.0f32;
        for (i, cell) in row
            .children
            .iter()
            .filter(|c| matches!(c.tag.as_str(), "td" | "th"))
            .enumerate()
        {
            let s = &cell.style;
            let inner = widths[i] - s.padding[1] - s.padding[3] - 2.0 * s.border_width;
            if inner <= 0.0 {
                return Err(Error("table cell has no usable width".into()));
            }
            let lines = lines_for(cell, inner, &self.fonts)?;
            count = count.max(lines.len());
            step = step.max(lines.iter().map(|l| l.height).fold(s.line_height, f32::max));
            pad = pad.max(s.padding[0] + s.padding[2] + 2.0 * s.border_width);
            cells.push(Cell {
                style: s.clone(),
                lines,
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
        })
    }
    fn ensure_header(&mut self, heads: &[Row], widths: &[f32], height: f32) -> Result<()> {
        if self.y + height > self.limit() + EPS && self.page_has_content() {
            self.new_page();
        }
        for row in heads {
            self.draw_row(row, widths, 0, row.line_count);
        }
        Ok(())
    }
    // A row fragment consumes at least one line. Every continuation starts with the header.
    fn draw_row(&mut self, row: &Row, widths: &[f32], start: usize, count: usize) {
        let height = count as f32 * row.step + row.pad;
        let mut x = self.content_x();
        let y = self.y;
        for (i, w) in widths.iter().enumerate() {
            if let Some(cell) = row.cells.get(i) {
                self.paint_box(x, y, *w, height, &cell.style);
                let text_x = x + cell.style.border_width + cell.style.padding[3];
                let mut line_y = y + cell.style.border_width + cell.style.padding[0];
                for line in cell.lines.iter().skip(start).take(count) {
                    self.draw_line(
                        line,
                        text_x,
                        line_y,
                        *w - cell.style.padding[1]
                            - cell.style.padding[3]
                            - 2.0 * cell.style.border_width,
                        &cell.style.text_align,
                    );
                    line_y += row.step;
                }
            }
            x += *w;
        }
        self.y += height;
    }
}
