use serpentype::css::{self, Color, Style};

fn renderer() -> serpentype::layout::Renderer {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    serpentype::layout::Renderer::new(fonts, ".".into(), true)
}

fn text_position(doc: &serpentype::layout::PreparedDocument, target: char) -> Option<(f32, f32)> {
    doc.pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            serpentype::layout::Item::Text { ch, x, y, .. } if *ch == target => Some((*x, *y)),
            _ => None,
        })
}

#[test]
fn css_colors_and_lengths() {
    assert_eq!(
        css::color("rebeccapurple"),
        Some(Color(102.0 / 255.0, 51.0 / 255.0, 153.0 / 255.0))
    );
    assert_eq!(css::color("rgb(255 0 0)"), Some(Color(1.0, 0.0, 0.0)));
    assert_eq!(css::color("hsl(120 100% 25%)"), Some(Color(0.0, 0.5, 0.0)));
    assert_eq!(css::color("rgb(NaN 0 0)"), None);
    assert_eq!(css::color("hsl(NaN 100% 25%)"), None);
    assert_eq!(css::computed_length("2em", 12.0), Some(24.0));
    assert_eq!(css::computed_length("1rem", 15.0), Some(12.0));
    assert_eq!(css::computed_length("calc(2em + 3pt)", 10.0), Some(23.0));
    assert_eq!(css::computed_length("calc(2em + 3%)", 10.0), None);
}

#[test]
fn style_accepts_contextual_units() {
    let mut style = Style::default();
    let mut warnings = vec![];
    css::apply(&mut style, "font-size", "2em", &mut warnings);
    css::apply(&mut style, "margin-left", "1em", &mut warnings);
    css::apply(&mut style, "border-width", "thin", &mut warnings);
    assert_eq!(style.font_size, 24.0);
    assert_eq!(style.margin[3], 24.0);
    assert_eq!(style.border_width, 0.75);
    assert!(warnings.is_empty());

    css::apply(&mut style, "width", "50%", &mut warnings);
    assert_eq!(style.width_percent, Some(0.5));
}

#[test]
fn visibility_hidden_keeps_block_layout_without_painting() {
    let renderer = renderer();
    let hidden = renderer
        .layout(
            "<p>A</p><p class='hidden'>HIDDEN</p><p>B</p>",
            "p { margin:0; line-height:20pt } .hidden { visibility:hidden; background:red }",
        )
        .unwrap();
    let visible = renderer
        .layout(
            "<p>A</p><p>HIDDEN</p><p>B</p>",
            "p { margin:0; line-height:20pt }",
        )
        .unwrap();
    let removed = renderer
        .layout(
            "<p>A</p><p class='removed'>HIDDEN</p><p>B</p>",
            "p { margin:0; line-height:20pt } .removed { display:none }",
        )
        .unwrap();

    let painted: String = hidden
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter_map(|item| match item {
            serpentype::layout::Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(painted, "AB");
    assert_eq!(text_position(&hidden, 'B'), text_position(&visible, 'B'));
    assert!(text_position(&hidden, 'B').unwrap().1 > text_position(&removed, 'B').unwrap().1);
    assert!(!hidden.pages.iter().flat_map(|page| &page.items).any(|item| {
        matches!(item, serpentype::layout::Item::Rect { fill: Some(color), .. } if *color == Color(1.0, 0.0, 0.0))
    }));

    let break_after_hidden = renderer
        .layout(
            "<p style='visibility:hidden'>HIDDEN</p><p style='break-before:page'>B</p>",
            "p { margin:0 }",
        )
        .unwrap();
    assert_eq!(break_after_hidden.page_count(), 2);
}

#[test]
fn inline_visibility_hidden_preserves_advance_without_painting() {
    let renderer = renderer();
    let hidden = renderer
        .layout(
            "<p>A<span style='visibility:hidden'>XX</span>B</p>",
            "p { margin:0 }",
        )
        .unwrap();
    let visible = renderer.layout("<p>AXXB</p>", "p { margin:0 }").unwrap();
    let painted: String = hidden.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            serpentype::layout::Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(painted, "AB");
    assert_eq!(text_position(&hidden, 'B'), text_position(&visible, 'B'));

    let restored = renderer
        .layout(
            "<p style='visibility:hidden'>A<span style='visibility:visible'>B</span>C</p>",
            "p { margin:0 }",
        )
        .unwrap();
    let restored_text: String = restored.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            serpentype::layout::Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(restored_text, "B");
}

#[test]
fn box_sizing_controls_declared_width() {
    let renderer = renderer();
    let rect_width = |box_sizing: &str| {
        let doc = renderer
            .layout(
                "<p>Text</p>",
                &format!(
                    "@page {{ size:200pt 200pt; margin:10pt }} p {{ margin:0; width:100pt; padding:10pt; border:2pt solid black; background:red; box-sizing:{box_sizing} }}"
                ),
            )
            .unwrap();
        doc.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                serpentype::layout::Item::Rect { w, .. } => Some(*w),
                _ => None,
            })
            .unwrap()
    };
    assert!((rect_width("content-box") - 124.0).abs() < 0.01);
    assert!((rect_width("border-box") - 100.0).abs() < 0.01);
}

#[test]
fn min_max_dimensions_constrain_block_geometry_without_clipping_text() {
    let renderer = renderer();
    let doc = renderer
        .layout(
            "<p class='max'>Wide</p><p class='min'>Tall</p><p>After</p>",
            "@page { size:200pt 250pt; margin:10pt } p { margin:0; box-sizing:border-box } .max { width:120pt; max-width:80pt; background:red } .min { width:40pt; min-width:70pt; min-height:50pt; background:blue }",
        )
        .unwrap();
    let rects: Vec<(f32, f32)> = doc.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            serpentype::layout::Item::Rect { w, h, .. } => Some((*w, *h)),
            _ => None,
        })
        .collect();
    assert!((rects[0].0 - 80.0).abs() < 0.01);
    assert!((rects[1].0 - 70.0).abs() < 0.01);
    assert!((rects[1].1 - 50.0).abs() < 0.01);
    assert!(text_position(&doc, 'A').unwrap().1 >= rects[1].1);
}

#[test]
fn percentage_width_uses_content_box() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let doc = renderer
        .layout(
            "<p style='width:50%;background:red'>Text</p>",
            "@page { size: 200pt 200pt; margin: 10pt }",
        )
        .unwrap();
    let width = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            serpentype::layout::Item::Rect { w, .. } => Some(*w),
            _ => None,
        })
        .unwrap();
    assert!((width - 90.0).abs() < 0.01);
}

#[test]
fn justify_and_indent_are_applied_before_pagination() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let html = "<p>one two three four five six seven eight nine ten</p>";
    let base = renderer
        .layout(
            html,
            "@page { size: 130pt 200pt; margin: 10pt } p { width: 100pt; margin: 0 }",
        )
        .unwrap();
    let justified = renderer.layout(html, "@page { size: 130pt 200pt; margin: 10pt } p { width: 100pt; margin: 0; text-align: justify; text-indent: 15pt }").unwrap();
    let first_x = |doc: &serpentype::layout::PreparedDocument| {
        doc.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                serpentype::layout::Item::Text { x, .. } => Some(*x),
                _ => None,
            })
            .unwrap()
    };
    assert!((first_x(&justified) - first_x(&base) - 15.0).abs() < 0.01);
    assert_eq!(justified.page_count(), 1);
}

#[test]
fn widows_move_text_to_next_page_before_drawing() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let doc = renderer
        .layout(
            "<p>A\nB\nC\nD\nE</p>",
            "@page { size: 200pt 100pt; margin: 10pt } p { white-space: pre; line-height: 20pt; margin: 0; widows: 2; orphans: 2 }",
        )
        .unwrap();
    let page_text: Vec<String> = doc
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    serpentype::layout::Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(page_text, vec!["ABC", "DE"]);
}

#[test]
fn orphans_move_paragraph_after_existing_content() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let doc = renderer
        .layout(
            "<p>P</p><p style='orphans:4'>A\nB\nC\nD</p>",
            "@page { size: 200pt 100pt; margin: 10pt } p { white-space: pre; line-height: 20pt; margin: 0 }",
        )
        .unwrap();
    let page_text: Vec<String> = doc
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    serpentype::layout::Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(page_text, vec!["P", "ABCD"]);
}

#[test]
fn relative_position_moves_paint_without_moving_normal_flow() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let plain = renderer.layout("<p>A</p><p>B</p>", "").unwrap();
    let moved = renderer
        .layout(
            "<p style='position:relative;left:15pt;top:8pt'>A</p><p>B</p>",
            "",
        )
        .unwrap();
    let position = |doc: &serpentype::layout::PreparedDocument, target| {
        doc.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                serpentype::layout::Item::Text { ch, x, y, .. } if *ch == target => Some((*x, *y)),
                _ => None,
            })
            .unwrap()
    };
    let (plain_x, plain_y) = position(&plain, 'A');
    let (moved_x, moved_y) = position(&moved, 'A');
    assert!((moved_x - plain_x - 15.0).abs() < 0.01);
    assert!((moved_y - plain_y - 8.0).abs() < 0.01);
    assert_eq!(position(&plain, 'B'), position(&moved, 'B'));
}

#[test]
fn grid_gap_offsets_tracks_and_rows() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let doc = renderer.layout("<div class='grid'><p>A</p><p>B</p><p>C</p></div>", ".grid { display:grid; grid-template-columns:repeat(2, 1fr); column-gap:20pt; row-gap:10pt } p { margin:0 }").unwrap();
    let xy = |target| {
        doc.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                serpentype::layout::Item::Text { ch, x, y, .. } if *ch == target => Some((*x, *y)),
                _ => None,
            })
            .unwrap()
    };
    assert!(
        (xy('B').0
            - xy('A').0
            - (doc.pages[0].style.width
                - doc.pages[0].style.margin[1]
                - doc.pages[0].style.margin[3]
                + 20.0)
                / 2.0)
            .abs()
            < 0.1
    );
    assert!((xy('C').1 - xy('A').1 - 24.4).abs() < 0.2);
}

#[test]
fn recto_break_inserts_blank_verso_page() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let doc = renderer
        .layout("<p>A</p><p style='break-before:right'>B</p>", "")
        .unwrap();
    let page_text: Vec<String> = doc
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    serpentype::layout::Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(page_text, vec!["A", "", "B"]);
}

#[test]
fn verso_break_uses_even_page_without_extra_blank() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let doc = renderer
        .layout("<p>A</p><p style='page-break-before:left'>B</p>", "")
        .unwrap();
    let page_text: Vec<String> = doc
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    serpentype::layout::Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(page_text, vec!["A", "B"]);
}

#[test]
fn page_pseudo_selectors_style_right_and_blank_pages() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let css = "@page { size:200pt 100pt; margin:15pt } @page :right { size:120pt 100pt } @page :blank { size:160pt 100pt; @top-center { content:'BLANK' } }";
    let doc = renderer
        .layout("<p>A</p><p style='break-before:right'>B</p>", css)
        .unwrap();
    assert_eq!(doc.pages.len(), 3);
    assert!((doc.pages[0].style.width - 120.0).abs() < 0.01);
    assert!((doc.pages[1].style.width - 160.0).abs() < 0.01);
    assert!((doc.pages[2].style.width - 120.0).abs() < 0.01);
    let blank_text: String = doc.pages[1]
        .items
        .iter()
        .filter_map(|item| match item {
            serpentype::layout::Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(blank_text, "BLANK");
}

#[test]
fn first_and_left_page_selectors_apply_to_their_pages() {
    let fonts = serpentype::font::FontRegistry::new(1_000_000);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/serpentype/assets/NotoSans-Regular.ttf"
            ),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let renderer = serpentype::layout::Renderer::new(fonts, ".".into(), true);
    let css = "@page { size:200pt 100pt; margin:15pt } @page :first { size:100pt 100pt } @page :left { size:150pt 100pt }";
    let doc = renderer
        .layout("<p>A</p><p style='break-before:left'>B</p>", css)
        .unwrap();
    assert_eq!(doc.pages.len(), 2);
    assert!((doc.pages[0].style.width - 100.0).abs() < 0.01);
    assert!((doc.pages[1].style.width - 150.0).abs() < 0.01);
}
