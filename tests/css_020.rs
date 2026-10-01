use serpentype::css::{self, Color, Style};
use serpentype::layout::Item;

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
fn inline_block_is_an_atomic_painted_box_inside_the_line() {
    let doc = renderer()
        .layout(
            "<p>A<span class='box'>B</span>C</p>",
            "@page { size:200pt 120pt; margin:10pt } p { margin:0 } \
             .box { display:inline-block; width:40pt; padding:4pt; border:1pt solid black; background:red }",
        )
        .unwrap();
    let (box_x, box_y, box_width, box_height) = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            serpentype::layout::Item::Rect {
                x,
                y,
                w,
                h,
                fill: Some(color),
                ..
            } if *color == Color(1.0, 0.0, 0.0) => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .expect("inline-block background");
    assert!((box_width - 50.0).abs() < 0.01);
    assert!(box_height > 8.0);
    let (b_x, b_y) = text_position(&doc, 'B').unwrap();
    let (c_x, _) = text_position(&doc, 'C').unwrap();
    assert!(b_x > box_x && b_x < box_x + box_width);
    assert!(b_y > box_y && b_y < box_y + box_height);
    assert!(c_x >= box_x + box_width - 0.01);
}

#[test]
fn inline_block_wraps_as_one_unit_and_can_format_multiple_internal_lines() {
    let doc = renderer()
        .layout(
            "<p>AAAA <span class='box'>one two</span>Z</p>",
            "p { margin:0; width:70pt } .box { display:inline-block; width:28pt; padding:2pt; background:red }",
        )
        .unwrap();
    let (_, a_y) = text_position(&doc, 'A').unwrap();
    let (_, o_y) = text_position(&doc, 'o').unwrap();
    let (_, t_y) = text_position(&doc, 't').unwrap();
    assert!(o_y > a_y);
    assert!(t_y > o_y);
    let (_, box_y, _, box_height) = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            serpentype::layout::Item::Rect {
                x,
                y,
                w,
                h,
                fill: Some(color),
                ..
            } if *color == Color(1.0, 0.0, 0.0) => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .unwrap();
    assert!(box_y > a_y - 20.0);
    assert!(box_height > 25.0);

    let mut shaped_renderer = renderer();
    shaped_renderer.experimental_shaping = true;
    let shaped = shaped_renderer
        .layout(
            "<p>A<span class='box'>office</span>Z</p>",
            "p { margin:0 } .box { display:inline-block; width:40pt; padding:2pt; background:red }",
        )
        .unwrap();
    assert!(shaped.pages[0]
        .items
        .iter()
        .any(|item| matches!(item, serpentype::layout::Item::Rect { .. })));
    assert!(text_position(&shaped, 'o').is_some());

    let linked = renderer()
        .layout(
            "<p><a href='https://example.com' style='display:inline-block;padding:2pt'>Link</a></p>",
            "p { margin:0 }",
        )
        .unwrap();
    assert_eq!(
        linked.pages[0]
            .items
            .iter()
            .filter(|item| matches!(item, serpentype::layout::Item::Link { .. }))
            .count(),
        1
    );
}

#[test]
fn manual_soft_hyphen_only_paints_when_used_for_a_break() {
    let base_renderer = renderer();
    let wide = base_renderer
        .layout(
            "<p>encyclo&shy;pedia</p>",
            "p { margin:0; width:160pt; overflow-wrap:normal; hyphens:manual }",
        )
        .unwrap();
    let narrow = base_renderer
        .layout(
            "<p>encyclo&shy;pedia</p>",
            "p { margin:0; width:55pt; overflow-wrap:normal; hyphens:manual }",
        )
        .unwrap();
    let text = |doc: &serpentype::layout::PreparedDocument| {
        doc.pages[0]
            .items
            .iter()
            .filter_map(|item| match item {
                serpentype::layout::Item::Text { ch, unicode, .. } => Some(
                    unicode
                        .as_deref()
                        .map(str::to_owned)
                        .unwrap_or_else(|| ch.to_string()),
                ),
                _ => None,
            })
            .collect::<String>()
    };
    assert_eq!(text(&wide), "encyclopedia");
    assert_eq!(text(&narrow), "encyclo-pedia");
    assert!(text_position(&narrow, 'p').unwrap().1 > text_position(&narrow, 'e').unwrap().1);

    let none = base_renderer
        .layout(
            "<p>encyclo&shy;pedia</p>",
            "p { margin:0; width:55pt; overflow-wrap:anywhere; hyphens:none }",
        )
        .unwrap();
    assert_eq!(text(&none), "encyclopedia");

    let mut shaped_renderer = renderer();
    shaped_renderer.experimental_shaping = true;
    let shaped = shaped_renderer
        .layout(
            "<p>encyclo&shy;pedia</p>",
            "p { margin:0; width:55pt; overflow-wrap:normal; hyphens:manual }",
        )
        .unwrap();
    assert_eq!(text(&shaped), "encyclo-pedia");

    let nearest = base_renderer
        .layout(
            "<p>prefix encyclo&shy;pedia</p>",
            "p { margin:0; width:85pt; overflow-wrap:normal; hyphens:manual }",
        )
        .unwrap();
    assert_eq!(text(&nearest), "prefix encyclo-pedia");
    assert_eq!(
        text_position(&nearest, 'p').unwrap().1,
        text_position(&nearest, '-').unwrap().1,
        "the nearest soft hyphen must win over an earlier space"
    );
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
fn percentage_edges_and_general_block_constraints_use_containing_block() {
    let doc = renderer().layout(
        "<div class='box'><p>X</p></div>",
        "@page { size:200pt 200pt; margin:10pt } .box { width:80%; min-width:100pt; max-width:120pt; margin-left:10%; padding-left:10%; background:red } p { margin:0 }",
    ).unwrap();
    let (x, w) = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::Rect {
                x,
                w,
                fill: Some(color),
                ..
            } if *color == Color(1.0, 0.0, 0.0) => Some((*x, *w)),
            _ => None,
        })
        .unwrap();
    assert!((x - 28.0).abs() < 0.2);
    assert!(
        (w - 138.0).abs() < 0.2,
        "content-box max-width plus percentage padding"
    );
    assert!(text_position(&doc, 'X').unwrap().0 > x + 17.0);
}

#[test]
fn flex_and_grid_container_min_max_widths_constrain_tracks() {
    let grid=renderer().layout("<div class='grid'><p>A</p><p>B</p></div>",".grid { display:grid; grid-template-columns:repeat(2, 1fr); width:160pt; max-width:100pt } p { margin:0 }").unwrap();
    let ax = text_position(&grid, 'A').unwrap().0;
    let bx = text_position(&grid, 'B').unwrap().0;
    assert!(
        (bx - ax - 50.0).abs() < 0.6,
        "grid tracks must use constrained width"
    );
    let flex = renderer()
        .layout(
            "<div class='flex'><p>A</p><p>B</p></div>",
            ".flex { display:flex; width:160pt; max-width:120pt } p { margin:0 }",
        )
        .unwrap();
    let ax = text_position(&flex, 'A').unwrap().0;
    let bx = text_position(&flex, 'B').unwrap().0;
    assert!(
        (bx - ax - 60.0).abs() < 0.6,
        "flex tracks must use constrained width"
    );
}

#[test]
fn alpha_opacity_and_modern_color_syntax_reach_pdf_graphics_state() {
    let (paint, alpha) = css::color_with_alpha("rgb(255 0 0 / 50%)").unwrap();
    assert_eq!(paint, Color(1.0, 0.0, 0.0));
    assert!((alpha - 0.5).abs() < 0.001);
    assert!((css::color_with_alpha("hsla(120,100%,50%,0.25)").unwrap().1 - 0.25).abs() < 0.001);
    let doc=renderer().layout("<div><p>Alpha</p></div>","div { background:rgba(255,0,0,.5); opacity:.5 } p { color:rgb(0 0 0 / 50%); margin:0 }").unwrap();
    let pdf = doc.to_pdf().unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/ExtGState"));
    assert!(doc.pages[0]
        .items
        .iter()
        .any(|item| matches!(item,Item::BeginOpacity(alpha) if (*alpha-0.25).abs()<0.01)));
}

#[test]
fn overflow_hidden_clips_without_changing_following_flow() {
    let doc=renderer().layout("<div class='clip'><p>A<br>B<br>C</p></div><p>Z</p>",".clip { height:20pt; overflow:hidden; border-radius:4pt } p { margin:0; line-height:14pt }").unwrap();
    assert!(doc.pages[0]
        .items
        .iter()
        .any(|item| matches!(item, Item::BeginClip { .. })));
    let z_y = text_position(&doc, 'Z').unwrap().1;
    let a_y = text_position(&doc, 'A').unwrap().1;
    assert!(
        z_y - a_y < 30.0,
        "following content advanced by hidden content: {a_y} {z_y}"
    );
    assert!(doc.to_pdf().unwrap().starts_with(b"%PDF-1.7"));
}

#[test]
fn sibling_margins_collapse_with_negative_values() {
    let doc=renderer().layout("<div><p style='margin:0 0 20pt'>A</p><p style='margin:10pt 0 0'>B</p><p style='margin:-5pt 0 0'>C</p></div>","p { line-height:10pt }").unwrap();
    let ay = text_position(&doc, 'A').unwrap().1;
    let by = text_position(&doc, 'B').unwrap().1;
    let cy = text_position(&doc, 'C').unwrap().1;
    let line_step = cy - by + 5.0;
    assert!(
        (by - ay - line_step - 20.0).abs() < 0.5,
        "positive margins collapse to the maximum: {ay} {by} {cy}"
    );
    assert!(
        (cy - by - (line_step - 5.0)).abs() < 0.5,
        "negative margin moves the following block upward"
    );
}

#[test]
fn parent_and_child_margins_collapse_through_empty_edges() {
    let doc=renderer().layout("<p style='margin:0'>A</p><div style='margin-top:10pt'><p style='margin:20pt 0 0'>B</p></div>","p { line-height:12pt }").unwrap();
    let ay = text_position(&doc, 'A').unwrap().1;
    let by = text_position(&doc, 'B').unwrap().1;
    assert!(
        (by - ay - 34.4).abs() < 0.6,
        "parent and first-child top margins must collapse: {ay} {by}"
    );
}

#[test]
fn per_side_dashed_dotted_and_rounded_borders_are_preserved() {
    let doc=renderer().layout("<div class='box'><p>X</p></div>",".box { border-top:2pt dashed red; border-right:3pt dotted blue; border-bottom:4pt solid green; border-left:1pt solid black; border-radius:2pt 4pt 6pt 8pt } p { margin:0 }").unwrap();
    let border = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::Border {
                widths,
                styles,
                radius,
                ..
            } => Some((widths, styles, radius)),
            _ => None,
        })
        .unwrap();
    assert_eq!(*border.0, [2.0, 3.0, 4.0, 1.0]);
    assert_eq!(border.1[0], "dashed");
    assert_eq!(border.1[1], "dotted");
    assert_eq!(*border.2, [2.0, 4.0, 6.0, 8.0]);
    assert!(doc.to_pdf().unwrap().starts_with(b"%PDF-1.7"));
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
