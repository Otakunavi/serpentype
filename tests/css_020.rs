use base64::Engine as _;
use image::ImageEncoder as _;
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
fn named_landscape_page_expands_an_auto_width_ancestor_frame() {
    let html = "<body><h1>Cover</h1><section class='wide'><table><tr>\
        <td>A1</td><td>B2</td><td>C3</td><td>D4</td><td>E5</td><td>F6</td>\
        </tr></table></section></body>";
    let css = "@page { size:A4; margin:15mm } @page wide { size:A4 landscape; margin:12mm } \
        table { width:100%; table-layout:fixed } td { border:1px solid black } \
        .wide { page:wide; break-before:page }";
    let doc = renderer().layout(html, css).unwrap();
    assert_eq!(doc.pages.len(), 2);
    assert!(doc.pages[1].style.width > doc.pages[1].style.height);
    let x_for = |target| {
        doc.pages[1]
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { ch, x, .. } if *ch == target => Some(*x),
                _ => None,
            })
            .unwrap()
    };
    assert!(x_for('F') - x_for('A') > 600.0);
}

#[test]
fn table_nested_in_inline_spans_keeps_table_layout_and_borders() {
    let html = "<div><span><span><table><tbody><tr><td>LEFT</td><td>RIGHT</td></tr>\
        <tr><td>LOWER</td><td>CELLS</td></tr></tbody></table></span></span></div>";
    let css = "@page { size:300pt 220pt; margin:15pt } \
        table { width:100%; border-collapse:collapse } \
        td { border:1pt solid black; padding:3pt }";
    let doc = renderer().layout(html, css).unwrap();
    let text = doc
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter_map(|item| match item {
            Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect::<String>();
    for expected in ["LEFT", "RIGHT", "LOWER", "CELLS"] {
        assert!(
            text.contains(expected),
            "missing table cell text {expected}"
        );
    }
    let border_rects = doc
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter(|item| matches!(item, Item::Rect { .. }))
        .count();
    assert!(
        border_rects >= 8,
        "expected painted table borders, found {border_rects}"
    );
}

#[test]
fn inline_wrapper_after_table_keeps_its_trailing_line_box() {
    let html =
        "<div><span><table><tbody><tr><td>R</td></tr></tbody></table></span></div><div>Z</div>";
    let css = "@page { size:200pt 200pt; margin:10pt } \
        div { margin:0; font-size:10pt; line-height:12pt } \
        table { border-collapse:collapse } td { border:0.1pt solid black; padding:0 }";
    let doc = renderer().layout(html, css).unwrap();
    let row_y = text_position(&doc, 'R').unwrap().1;
    let following_y = text_position(&doc, 'Z').unwrap().1;
    assert!(
        (following_y - row_y - 24.0).abs() < 0.5,
        "expected the table row and trailing inline line box, got {}pt",
        following_y - row_y
    );
}

#[test]
fn trailing_collapsible_space_does_not_split_a_fitting_phrase() {
    let html = "<div>По результатам камерального контроля</div>";
    let css = "@page { size:200pt 120pt; margin:10pt } \
        div { width:77.387pt; font-size:10pt; line-height:12.5pt; margin:0 }";
    let doc = renderer().layout(html, css).unwrap();
    let first_word_y = text_position(&doc, 'П').unwrap().1;
    let second_word_y = text_position(&doc, 'р').unwrap().1;
    assert!(
        (first_word_y - second_word_y).abs() < 0.1,
        "a trailing collapsible space split a phrase that fits: {first_word_y} vs {second_word_y}"
    );
}

#[test]
fn flex_inline_items_use_their_intrinsic_widths() {
    let html = "<div class='flex'><span class='badge'>FLEX</span><b>middle</b><i>end</i></div>";
    let css = "@page { size:300pt 100pt; margin:10pt } body { margin:0 } \
        .flex { display:flex; width:240pt; justify-content:space-between; align-items:center } \
        .badge { display:inline-block; padding:2pt 4pt; border-radius:5pt; background:#207448 }";
    let doc = renderer().layout(html, css).unwrap();
    let badge_width = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::RoundedRect { w, .. } => Some(*w),
            _ => None,
        })
        .expect("expected the badge background");
    assert!(
        badge_width < 80.0,
        "expected intrinsic badge width, got {badge_width}pt"
    );
}

#[test]
fn inline_multicolumn_text_uses_balanced_side_by_side_columns() {
    let html = "<div class='columns'>LEFTCOLUMN words repeat across the page. \
        RIGHTCOLUMN words repeat across the page. LEFTCOLUMN words repeat across the page. \
        RIGHTCOLUMN words repeat across the page. LEFTCOLUMN words repeat across the page. \
        RIGHTCOLUMN words repeat across the page. LEFTCOLUMN words repeat across the page. \
        RIGHTCOLUMN words repeat across the page.</div>";
    let css = "@page { size:180pt 600pt; margin:10pt } \
        .columns { column-count:2; column-gap:10pt }";
    let doc = renderer().layout(html, css).unwrap();
    assert_eq!(doc.pages.len(), 1);
    let mut lines = std::collections::HashMap::<i32, (f32, f32)>::new();
    for item in &doc.pages[0].items {
        if let Item::Text { x, y, .. } = item {
            let entry = lines.entry((*y * 10.0).round() as i32).or_insert((*x, *x));
            entry.0 = entry.0.min(*x);
            entry.1 = entry.1.max(*x);
        }
    }
    assert!(lines.values().any(|(min_x, max_x)| max_x - min_x > 60.0));
}

fn all_text_positions(
    doc: &serpentype::layout::PreparedDocument,
    target: char,
) -> Vec<(usize, f32, f32)> {
    doc.pages
        .iter()
        .enumerate()
        .flat_map(|(page, content)| {
            content.items.iter().filter_map(move |item| match item {
                Item::Text { ch, x, y, .. } if *ch == target => Some((page, *x, *y)),
                _ => None,
            })
        })
        .collect()
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
    css::apply(&mut style, "object-position", "10pt 25%", &mut warnings);
    assert_eq!(style.object_position, [0.0, 0.25]);
    assert_eq!(style.object_position_offset, [10.0, 0.0]);
}

#[test]
fn positioning_flex_grid_and_transform_properties_parse() {
    let mut style = Style::default();
    let mut warnings = vec![];
    for (property, value) in [
        ("position", "absolute"),
        ("z-index", "-2"),
        ("transform", "translate(10pt, 5pt) scale(2) rotate(90deg)"),
        ("flex-direction", "row-reverse"),
        ("flex-wrap", "wrap"),
        ("flex-grow", "2"),
        ("flex-shrink", "0.5"),
        ("flex-basis", "25%"),
        ("align-self", "end"),
        ("align-content", "space-between"),
        (
            "grid-template-columns",
            "40pt 25% minmax(20pt, 1fr) repeat(2, auto)",
        ),
        ("grid-template-rows", "20pt 1fr"),
        ("grid-column", "2 / span 2"),
        ("grid-row", "1 / span 2"),
    ] {
        css::apply(&mut style, property, value, &mut warnings);
    }
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(style.position, "absolute");
    assert_eq!(style.z_index, -2);
    assert_ne!(style.transform, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    assert_eq!(style.flex_direction, "row-reverse");
    assert_eq!(style.flex_basis_percent, Some(0.25));
    assert_eq!(style.grid_template_columns.len(), 5);
    assert_eq!(style.grid_column_start, Some(2));
    assert_eq!(style.grid_column_span, 2);
}

#[test]
fn absolute_and_fixed_positioning_are_out_of_flow_layered_and_repeated() {
    let doc = renderer()
        .layout(
            "<div class='fixed'>F</div><div class='host'><div class='behind'>X</div><p class='front'>A</p></div><p style='break-before:page'>B</p>",
            "@page { size:180pt 120pt; margin:10pt } \
             p { margin:0; line-height:18pt } \
             .fixed { position:fixed; top:2pt; right:3pt; width:20pt; z-index:3 } \
             .host { position:relative; width:100pt; height:50pt; overflow:hidden } \
             .behind { position:absolute; left:20pt; top:10pt; width:30pt; height:20pt; background:red; z-index:-1 } \
             .front { position:relative; z-index:2; background:blue }",
        )
        .unwrap();
    assert_eq!(doc.page_count(), 2);
    let fixed = all_text_positions(&doc, 'F');
    assert_eq!(fixed.len(), 2);
    assert_eq!(fixed[0].0, 0);
    assert_eq!(fixed[1].0, 1);
    let x = text_position(&doc, 'X').unwrap();
    let a = text_position(&doc, 'A').unwrap();
    assert!(x.0 > a.0 + 15.0);
    assert!(doc.pages[0].items.iter().any(
        |item| matches!(item, Item::BeginClip { w, h, .. } if (*w - 100.0).abs() < 0.1 && *h > 40.0)
    ));
    let red = doc.pages[0].items.iter().position(|item| {
        matches!(
            item,
            Item::Rect {
                fill: Some(Color(1.0, 0.0, 0.0)),
                ..
            }
        )
    });
    let a_item = doc.pages[0]
        .items
        .iter()
        .position(|item| matches!(item, Item::Text { ch: 'A', .. }));
    assert!(red.unwrap() < a_item.unwrap());
    let blue = doc.pages[0].items.iter().position(|item| {
        matches!(
            item,
            Item::Rect {
                fill: Some(Color(0.0, 0.0, 1.0)),
                ..
            }
        )
    });
    assert!(blue.unwrap() > red.unwrap());
}

#[test]
fn translate_scale_and_rotate_reach_the_pdf_and_transform_links() {
    let doc = renderer()
        .layout(
            "<p><a href='https://example.test'>T</a></p>",
            "p { margin:0; width:40pt; transform:translate(12pt, 7pt) scale(1.5) rotate(15deg) }",
        )
        .unwrap();
    assert!(doc.pages[0]
        .items
        .iter()
        .any(|item| matches!(item, Item::BeginTransform(matrix) if (matrix[4].abs() + matrix[5].abs()) > 1.0)));
    let pdf = doc.to_pdf().unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/Subtype /Link"));
}

#[test]
fn flex_wrap_reverse_growth_alignment_and_percentage_basis_are_applied() {
    let doc = renderer()
        .layout(
            "<div class='grow'><div class='one'>A</div><div class='two'>B</div></div>\
             <div class='wrap'><div>C</div><div>D</div><div>E</div></div>\
             <div class='shrink'><div class='no-shrink'>G</div><div>H</div></div>\
             <div class='column'><div>I</div><div>J</div><div>K</div></div>",
            "@page { size:220pt 400pt; margin:10pt } \
             .grow,.wrap,.shrink,.column { display:flex; width:120pt; gap:10pt } \
             .grow > div { flex-basis:20pt; margin:0; background:red } \
             .grow .one { flex-grow:1; height:30pt } .grow .two { flex-grow:2; height:10pt; align-self:end } \
             .wrap { height:80pt; align-content:space-between; flex-wrap:wrap; flex-direction:row-reverse; margin-top:10pt } \
             .wrap > div { width:45%; margin:0 } \
             .shrink { width:80pt } .shrink > div { flex-basis:60pt; margin:0; background:blue } \
             .shrink .no-shrink { flex-shrink:0 } \
             .column { flex-direction:column-reverse; gap:0 } .column > div { margin:0 }",
        )
        .unwrap();
    let (ax, ay) = text_position(&doc, 'A').unwrap();
    let (bx, by) = text_position(&doc, 'B').unwrap();
    assert!(bx > ax + 40.0);
    assert!(by > ay + 10.0);
    let red_widths: Vec<_> = doc.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect {
                w,
                fill: Some(Color(1.0, 0.0, 0.0)),
                ..
            } => Some(*w),
            _ => None,
        })
        .collect();
    assert_eq!(red_widths.len(), 2);
    assert!(red_widths[1] > red_widths[0]);
    let (_, cy) = text_position(&doc, 'C').unwrap();
    let (dx, _) = text_position(&doc, 'D').unwrap();
    let (ex, ey) = text_position(&doc, 'E').unwrap();
    assert!(ex < dx, "row-reverse should put E before D");
    assert!(
        ey < cy,
        "the reversed first items should occupy the first flex line"
    );
    assert!(
        cy - ey > 50.0,
        "align-content should distribute wrapped lines"
    );
    let blue_widths: Vec<_> = doc.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect {
                w,
                fill: Some(Color(0.0, 0.0, 1.0)),
                ..
            } => Some(*w),
            _ => None,
        })
        .collect();
    assert_eq!(blue_widths.len(), 2);
    assert!(blue_widths[0] > blue_widths[1] + 40.0);
    assert!(text_position(&doc, 'K').unwrap().1 < text_position(&doc, 'I').unwrap().1);
}

#[test]
fn grid_resolves_track_functions_explicit_placement_spans_and_alignment() {
    let doc = renderer()
        .layout(
            "<div class='grid'><div class='span'>S</div><div class='auto'>A</div><div class='last'>L</div></div>",
            "@page { size:260pt 180pt; margin:10pt } \
             .grid { display:grid; width:200pt; height:100pt; grid-template-columns:40pt 25% minmax(20pt,1fr); grid-template-rows:30pt 40pt; gap:10pt; align-items:center; align-content:space-between; justify-items:center } \
             .grid > div { margin:0; width:20pt } \
             .span { grid-column:2 / span 2; grid-row:1 } \
             .last { grid-column:3; grid-row:2; align-self:end; justify-self:end }",
        )
        .unwrap();
    let (sx, sy) = text_position(&doc, 'S').unwrap();
    let (ax, _) = text_position(&doc, 'A').unwrap();
    let (lx, ly) = text_position(&doc, 'L').unwrap();
    assert!(sx > ax + 35.0);
    assert!(lx > sx + 40.0);
    assert!(ly > sy + 45.0);

    let auto = renderer()
        .layout(
            "<div class='grid'><div>A</div><div>B</div></div>",
            ".grid { display:grid; width:100pt; grid-template-columns:repeat(2,auto) } \
             .grid > div { margin:0; background:red }",
        )
        .unwrap();
    let widths: Vec<_> = auto.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect {
                w,
                fill: Some(Color(1.0, 0.0, 0.0)),
                ..
            } => Some(*w),
            _ => None,
        })
        .collect();
    assert_eq!(widths.len(), 2);
    assert!(widths.iter().all(|width| (*width - 50.0).abs() < 0.1));
}

#[test]
fn flex_and_grid_fragment_only_at_supported_boundaries() {
    let row_flex = renderer()
        .layout(
            "<p>lead</p><div class='f'><div>A</div><div>B</div><div>C</div></div>",
            "@page { size:160pt 90pt; margin:10pt } p { height:35pt; margin:0 } \
             .f { display:flex; flex-wrap:wrap; width:120pt } .f > div { width:60pt; height:25pt }",
        )
        .unwrap();
    assert!(row_flex.page_count() >= 2);
    assert_eq!(
        all_text_positions(&row_flex, 'A')[0].0,
        all_text_positions(&row_flex, 'B')[0].0
    );

    let column = renderer()
        .layout(
            "<div class='f'><div>A</div><div>B</div><div>C</div></div>",
            "@page { size:120pt 70pt; margin:10pt } .f { display:flex; flex-direction:column } .f > div { height:25pt }",
        )
        .unwrap();
    assert!(column.page_count() >= 2);

    let grid = renderer()
        .layout(
            "<div class='g'><div>A</div><div>B</div><div>C</div><div>D</div></div>",
             "@page { size:140pt 80pt; margin:10pt } .g { display:grid; grid-template-columns:repeat(2,1fr); grid-template-rows:repeat(2,35pt) }",
        )
        .unwrap();
    assert!(grid.page_count() >= 2);
    assert_eq!(
        all_text_positions(&grid, 'C')[0].0,
        all_text_positions(&grid, 'D')[0].0
    );
}

#[test]
fn object_fit_and_position_control_replaced_image_geometry() {
    let renderer = renderer();
    let contain = renderer
        .layout(
            "<img src='tests/fixtures/blue.png'>",
            "img { width:100pt; height:100pt; object-fit:contain; object-position:bottom }",
        )
        .unwrap();
    let (contain_y, contain_w, contain_h) = contain.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::Image { y, w, h, .. } => Some((*y, *w, *h)),
            _ => None,
        })
        .unwrap();
    assert!((contain_w - 100.0).abs() < 0.02);
    assert!((contain_h - 75.0).abs() < 0.02);
    assert!((contain_y - (contain.page_style.margin[0] + 25.0)).abs() < 0.02);

    let cover = renderer
        .layout(
            "<img src='tests/fixtures/blue.png'>",
            "img { width:100pt; height:100pt; object-fit:cover; object-position:right center }",
        )
        .unwrap();
    let (cover_x, cover_w, cover_h) = cover.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::Image { x, w, h, .. } => Some((*x, *w, *h)),
            _ => None,
        })
        .unwrap();
    assert!((cover_w - 133.333).abs() < 0.02);
    assert!((cover_h - 100.0).abs() < 0.02);
    assert!((cover_x - (cover.page_style.margin[3] - 33.333)).abs() < 0.02);
    assert!(cover.pages[0]
        .items
        .iter()
        .any(|item| matches!(item, Item::BeginClip { .. })));
}

#[test]
fn raster_images_can_be_downsampled_to_a_configured_dpi() {
    let pixels = vec![128; 100 * 50 * 3];
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, 100, 50, image::ExtendedColorType::Rgb8)
        .unwrap();
    let uri = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    let mut renderer = renderer();
    renderer.max_image_dpi = Some(72.0);
    let doc = renderer
        .layout(
            &format!("<img src='{uri}'>"),
            "img { width:10pt; height:5pt }",
        )
        .unwrap();
    let dimensions = doc.pages[0].items.iter().find_map(|item| match item {
        Item::Image { data, .. } => Some((data.width, data.height)),
        _ => None,
    });
    assert_eq!(dimensions, Some((10, 5)));
}

#[test]
fn supported_svg_is_exported_as_a_vector_form() {
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg' width='20' height='10'><path d='M0 0L20 10' stroke='red'/></svg>";
    let uri = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    );
    let pdf = renderer()
        .layout(&format!("<img src='{uri}'>"), "")
        .unwrap()
        .to_pdf()
        .unwrap();
    assert!(pdf
        .windows(b"/Subtype /Form".len())
        .any(|w| w == b"/Subtype /Form"));
    assert!(!pdf
        .windows(b"/Subtype /Image".len())
        .any(|w| w == b"/Subtype /Image"));
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
fn named_page_selectors_compose_with_page_side_and_first_selectors() {
    let doc = renderer()
        .layout(
            "<p style='page: chapter'>A</p><p style='page: chapter; break-before:right'>B</p>",
            "@page { size:100pt 100pt; margin:10pt } @page chapter { size:200pt 100pt } @page chapter:left { size:300pt 100pt } @page chapter:first { size:250pt 100pt } @page chapter:blank:left { size:350pt 100pt }",
        )
        .unwrap();
    assert_eq!(doc.pages.len(), 3);
    assert!((doc.pages[0].style.width - 250.0).abs() < 0.01);
    assert!((doc.pages[1].style.width - 350.0).abs() < 0.01);
    assert!((doc.pages[2].style.width - 200.0).abs() < 0.01);
}

#[test]
fn break_before_side_wins_over_conflicting_previous_break_after() {
    let doc = renderer()
        .layout(
            "<p style='break-after:left'>A</p><p style='break-before:right'>B</p>",
            "@page { size:200pt 100pt; margin:10pt }",
        )
        .unwrap();
    assert_eq!(doc.pages.len(), 3);
    assert!(doc.pages[1].items.is_empty());
    let last_text: String = doc.pages[2]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(last_text, "B");
}

#[test]
fn nested_counters_strings_and_running_elements_resolve_in_margin_boxes() {
    let doc = renderer()
        .layout(
            "<div class='running'>Running head</div><section><p class='increment'>Counted</p><h1>Chapter title</h1></section>",
            "@page { size:240pt 160pt; margin:20pt; @top-center { content: element(running) } @bottom-center { content: counters(section, '.') ' / ' string(chapter) } } body { counter-reset: section 0 } section { counter-reset: section 2 } .increment { counter-increment: section 3 } h1 { string-set: chapter content() } .running { position: running(running) }",
        )
        .unwrap();
    let text: String = doc.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert!(text.contains("Running head"), "{text}");
    assert!(text.contains("0.5 / Chapter title"), "{text}");
    assert!(!text.contains("Running headRunning head"), "{text}");
}

#[test]
fn bleed_and_crop_marks_expand_the_pdf_page_and_retain_trim_geometry() {
    let doc = renderer()
        .layout(
            "<p>Print</p>",
            "@page { size:200pt 100pt; margin:10pt; bleed:5pt; marks:crop }",
        )
        .unwrap();
    assert_eq!(doc.pages[0].style.bleed, 5.0);
    assert!(doc.pages[0].style.crop_marks);
    let pdf = doc.to_pdf().unwrap();
    let source = String::from_utf8_lossy(&pdf);
    assert!(source.contains("/MediaBox [0 0 234.000 134.000]"));
    assert!(source.contains("/TrimBox [17.000 17.000 217.000 117.000]"));
    assert!(source.contains("/BleedBox [12.000 12.000 222.000 122.000]"));
    assert!(source.contains("/BleedBox"));
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

#[test]
fn block_html_tags_with_inline_display_stay_in_the_surrounding_line() {
    let doc = renderer()
        .layout(
            "<div><span>A </span><div style='display:inline'><span>REG-2026</span></div><span> Z</span></div>",
            "@page { size:200pt 100pt; margin:10pt } div { margin:0; font-size:10pt; line-height:12pt }",
        )
        .unwrap();
    let ay = text_position(&doc, 'A').unwrap().1;
    let reg_y = text_position(&doc, 'R').unwrap().1;
    let zy = text_position(&doc, 'Z').unwrap().1;
    assert!(
        (reg_y - ay).abs() < 0.1,
        "inline div moved to another line: {ay} {reg_y}"
    );
    assert!(
        (zy - ay).abs() < 0.1,
        "content after inline div moved to another line: {ay} {zy}"
    );
}

#[test]
fn block_html_tags_with_inline_block_display_stay_in_the_surrounding_line() {
    let doc = renderer()
        .layout(
            "<div><span>A </span><div style='display:inline-block'><span>REG-2026</span></div><span> Z</span></div>",
            "@page { size:200pt 100pt; margin:10pt } div { margin:0; font-size:10pt; line-height:12pt }",
        )
        .unwrap();
    let ay = text_position(&doc, 'A').unwrap().1;
    let zy = text_position(&doc, 'Z').unwrap().1;
    assert!(
        (zy - ay).abs() < 0.1,
        "content after inline-block div moved to another line: {ay} {zy}"
    );
}

#[test]
fn unitless_line_height_scales_with_inherited_font_size() {
    let doc = renderer()
        .layout(
            "<div style='font-size:12pt; line-height:1.5'><span style='font-size:20pt'>A<br>B</span></div>",
            "@page { size:200pt 200pt; margin:10pt }",
        )
        .unwrap();
    let ay = text_position(&doc, 'A').unwrap().1;
    let by = text_position(&doc, 'B').unwrap().1;
    assert!(
        (by - ay - 30.0).abs() < 0.1,
        "expected 30pt line advance, got {}",
        by - ay
    );
}

#[test]
fn glyph_baselines_use_font_vertical_metrics_without_expanding_css_line_height() {
    let html = "<p>A</p><p>B</p>";
    let css = "@page { size:200pt 100pt; margin:0 } body, p { margin:0 } \
        p { font-size:10pt; line-height:12pt }";
    let doc = renderer().layout(html, css).unwrap();
    let face = ttf_parser::Face::parse(
        include_bytes!("../serpentype/assets/NotoSans-Regular.ttf"),
        0,
    )
    .unwrap();
    let ascent = face.ascender() as f32 / face.units_per_em() as f32 * 10.0;
    let descent = -(face.descender() as f32 / face.units_per_em() as f32 * 10.0);
    let expected_baseline = ascent + (12.0 - ascent - descent) / 2.0;
    let a = text_position(&doc, 'A').unwrap().1;
    let b = text_position(&doc, 'B').unwrap().1;

    assert!(
        (a - expected_baseline).abs() < 0.05,
        "{a} != {expected_baseline}"
    );
    assert!(
        (b - a - 12.0).abs() < 0.05,
        "line height changed: {a} -> {b}"
    );
}

#[test]
fn block_transform_origin_uses_the_auto_width_border_box() {
    let doc = renderer()
        .layout(
            "<p class='rotated'>A</p>",
            "@page { size:200pt 100pt; margin:10pt } \
             p { margin:0; font-size:10pt; line-height:12pt } \
             .rotated { transform:rotate(90deg) }",
        )
        .unwrap();
    let matrix = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            Item::BeginTransform(matrix) => Some(*matrix),
            _ => None,
        })
        .unwrap();

    assert!(
        (matrix[4] - 116.0).abs() < 0.1,
        "transform origin should center the 180pt auto-width block, got {matrix:?}"
    );
    assert!(
        (matrix[5] + 84.0).abs() < 0.1,
        "transform origin should center the 12pt line box, got {matrix:?}"
    );
}

#[test]
fn empty_block_with_br_preserves_its_line_box() {
    let doc = renderer()
        .layout(
            "<div>A</div><div><br></div><div>B</div>",
            "@page { size:200pt 200pt; margin:10pt } div { margin:0; font-size:10pt; line-height:12pt }",
        )
        .unwrap();
    let ay = text_position(&doc, 'A').unwrap().1;
    let by = text_position(&doc, 'B').unwrap().1;
    assert!(
        (by - ay - 24.0).abs() < 0.1,
        "expected one blank line, got {}pt",
        by - ay
    );
}
