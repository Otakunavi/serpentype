use serpentype::css::Color;
use serpentype::font::FontRegistry;
use serpentype::layout::{Item, Renderer};

fn renderer() -> Renderer {
    let fonts = FontRegistry::new(1_000_000);
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
    Renderer::new(fonts, ".".into(), true)
}

fn text_x(document: &serpentype::layout::PreparedDocument, target: char) -> f32 {
    document
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Item::Text { ch, x, .. } if *ch == target => Some(*x),
            _ => None,
        })
        .unwrap()
}

fn text_y(document: &serpentype::layout::PreparedDocument, target: char) -> f32 {
    document
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Item::Text { ch, y, .. } if *ch == target => Some(*y),
            _ => None,
        })
        .unwrap()
}

#[test]
fn html_table_cells_middle_align_by_default_and_headers_keep_start_alignment() {
    let html = "<table><tr><th>H</th><th>J</th></tr>\
                <tr><td>A</td><td>B<br>C<br>D</td></tr></table>";
    let css = "@page { size:200pt 180pt; margin:10pt } \
        table { width:120pt; table-layout:fixed; border-collapse:collapse } \
        th, td { padding:0; border:none; font-size:10pt; line-height:12pt }";
    let doc = renderer().layout(html, css).unwrap();

    assert!(
        text_x(&doc, 'H') < 15.0,
        "the default <th> text should remain start-aligned"
    );
    assert!(
        (text_y(&doc, 'A') - text_y(&doc, 'C')).abs() < 0.1,
        "a short <td> should align to the middle line of a taller row"
    );
}

#[test]
fn colspan_aligns_the_following_cell_with_its_column() {
    let html = "<table><tr><td colspan='2'>H</td><td>E</td></tr>\
                <tr><td>A</td><td>B</td><td>C</td></tr></table>";
    let doc = renderer()
        .layout(html, "table { width: 180pt } td { padding: 2pt }")
        .unwrap();
    let x = |target| {
        doc.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { ch, x, .. } if *ch == target => Some(*x),
                _ => None,
            })
            .unwrap()
    };
    assert!((x('E') - x('C')).abs() < 0.1);
    assert!(x('A') < x('B') && x('B') < x('C'));
}

#[test]
fn invalid_colspan_fails_without_losing_cells() {
    match renderer().layout(
        "<table><tr><td colspan='0'>A</td><td>B</td></tr></table>",
        "",
    ) {
        Err(error) => assert!(error.to_string().contains("colspan")),
        Ok(_) => panic!("invalid colspan was accepted"),
    }
}

#[test]
fn fixed_table_layout_uses_first_row_widths() {
    let html = "<table><tr><td style='width:20pt'>A</td><td>B</td></tr>\
                <tr><td>abcdefghijklmnopqrstuvwxyz0123456789</td><td>C</td></tr></table>";
    let css = "table { width: 180pt } td { padding: 1pt; overflow-wrap: anywhere }";
    let auto = renderer()
        .layout(html, &format!("{css} table {{ table-layout: auto }}"))
        .unwrap();
    let fixed = renderer()
        .layout(html, &format!("{css} table {{ table-layout: fixed }}"))
        .unwrap();
    let b_x = |doc: &serpentype::layout::PreparedDocument| {
        doc.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { ch: 'B', x, .. } => Some(*x),
                _ => None,
            })
            .unwrap()
    };
    assert!(b_x(&fixed) + 5.0 < b_x(&auto));
}

#[test]
fn separate_table_borders_apply_horizontal_and_vertical_spacing() {
    let document = renderer()
        .layout(
            "<table><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></table>",
            "table { width:140pt; table-layout:fixed; border-collapse:separate; border-spacing:10pt 6pt } \
             td { border:1pt solid black; background:white; padding:1pt; line-height:12pt }",
        )
        .unwrap();
    let rects = document.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rects.len(), 4);
    assert!((rects[1].0 - (rects[0].0 + rects[0].2) - 10.0).abs() < 0.1);
    assert!((rects[2].1 - (rects[0].1 + rects[0].3) - 6.0).abs() < 0.1);
}

#[test]
fn collapsed_table_ignores_border_spacing() {
    let document = renderer()
        .layout(
            "<table><tr><td>A</td><td>B</td></tr></table>",
            "table { width:140pt; table-layout:fixed; border-collapse:collapse; border-spacing:20pt } \
             td { border:1pt solid black; background:white; padding:1pt }",
        )
        .unwrap();
    let rects = document.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect {
                x,
                w,
                fill: Some(color),
                ..
            } if *color == Color(1.0, 1.0, 1.0) => Some((*x, *w)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rects.len(), 2);
    assert!((rects[1].0 - (rects[0].0 + rects[0].1)).abs() < 0.1);
}

#[test]
fn separate_border_spacing_participates_in_table_pagination() {
    let rows = (0..14)
        .map(|index| format!("<tr><td>R{index:02}</td></tr>"))
        .collect::<String>();
    let document = renderer()
        .layout(
            &format!("<table><thead><tr><th>HEAD</th></tr></thead><tbody>{rows}</tbody></table>"),
            "@page { size:140pt 100pt; margin:10pt } \
             table { width:120pt; border-collapse:separate; border-spacing:0 3pt } \
             td, th { border:0.5pt solid black; padding:1pt; font-size:8pt; line-height:10pt }",
        )
        .unwrap();
    assert!(document.page_count() > 1);
    let combined = document
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(combined.iter().all(|page| page.contains("HEAD")));
    let all = combined.join("|");
    for index in 0..14 {
        assert_eq!(all.matches(&format!("R{index:02}")).count(), 1);
    }
}

#[test]
fn cell_vertical_align_positions_short_content() {
    let document = renderer()
        .layout(
            "<table><tr><td style='vertical-align:bottom'>A</td>\
             <td style='vertical-align:middle'>B</td><td>C<br>D<br>E</td></tr></table>",
            "table { width: 180pt; table-layout: fixed } \
             td { padding: 2pt; line-height: 14pt }",
        )
        .unwrap();
    let y = |target| {
        document.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { ch, y, .. } if *ch == target => Some(*y),
                _ => None,
            })
            .unwrap()
    };
    assert!(y('A') > y('B'));
    assert!(y('B') > y('C'));
    assert!((y('A') - y('E')).abs() < 3.0);
}

#[test]
fn repeating_header_without_data_is_rejected() {
    match renderer().layout("<table><thead><tr><th>Header</th></tr></thead></table>", "") {
        Err(error) => assert!(error.to_string().contains("header has no data")),
        Ok(_) => panic!("table header without data was accepted"),
    }
}

#[test]
fn header_waits_for_at_least_one_data_line() {
    let doc = renderer()
        .layout(
            "<p>P</p><table><thead><tr><th>H</th></tr></thead><tbody><tr><td>A</td></tr></tbody></table>",
            "@page { size: 200pt 100pt; margin: 10pt } p { line-height: 55pt; margin: 0 }",
        )
        .unwrap();
    let page_text: Vec<String> = doc
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(page_text, vec!["P", "HA"]);
}

#[test]
fn repeating_groups_do_not_leave_a_header_without_its_fitting_row() {
    let document = renderer()
        .layout(
            "<p>P</p><table><thead><tr><th>HEAD</th></tr></thead>\
             <tbody><tr><td>A<br>B</td></tr></tbody>\
             <tfoot><tr><td>FOOT</td></tr></tfoot></table>",
            "@page { size: 160pt 100pt; margin: 10pt } \
             p { line-height: 40pt; margin: 0 } \
             th, td { line-height: 12pt; padding: 0 }",
        )
        .unwrap();
    let page_text = document
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert_eq!(page_text, vec!["P", "HEADABFOOT"]);
}

#[test]
fn rowspan_occupies_columns_and_keeps_its_group_together() {
    let html = "<p>P</p><table><tbody>\
                <tr><td rowspan='2'>A</td><td>B</td></tr>\
                <tr><td>C</td></tr>\
                <tr><td>D</td><td>E</td></tr>\
                </tbody></table>";
    let css = "@page { size: 160pt 100pt; margin: 10pt } \
               p { margin: 0; line-height: 55pt } \
               table { width: 140pt; table-layout: fixed } \
               td { border: 0.5pt solid black; padding: 1pt; line-height: 20pt }";
    let document = renderer().layout(html, css).unwrap();
    assert_eq!(document.page_count(), 2);

    let page_text = |index: usize| {
        document.pages[index]
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text { ch, .. } => Some(*ch),
                _ => None,
            })
            .collect::<String>()
    };
    assert_eq!(page_text(0), "P");
    assert_eq!(page_text(1), "ABCDE");

    let x = |target| {
        document.pages[1]
            .items
            .iter()
            .find_map(|item| match item {
                Item::Text { ch, x, .. } if *ch == target => Some(*x),
                _ => None,
            })
            .unwrap()
    };
    assert!((x('B') - x('C')).abs() < 0.1);
    assert!((x('A') - x('D')).abs() < 0.1);
    assert!((x('B') - x('E')).abs() < 0.1);
}

#[test]
fn invalid_rowspan_is_a_controlled_error() {
    match renderer().layout(
        "<table><tr><td rowspan='0'>A</td><td>B</td></tr></table>",
        "",
    ) {
        Err(error) => assert!(error.to_string().contains("rowspan")),
        Ok(_) => panic!("invalid rowspan was accepted"),
    }
}

#[test]
fn rowspan_cannot_cross_its_row_group() {
    match renderer().layout(
        "<table><tbody><tr><td rowspan='2'>A</td></tr></tbody>\
         <tbody><tr><td>B</td></tr></tbody></table>",
        "",
    ) {
        Err(error) => assert!(error.to_string().contains("rowspan")),
        Ok(_) => panic!("rowspan crossing tbody groups was accepted"),
    }
}

#[test]
fn oversized_rowspan_group_fragments_without_clipping_text() {
    let long = (0..120)
        .map(|index| format!("TOKEN{index:03}"))
        .collect::<Vec<_>>()
        .join(" ");
    let html = format!(
        "<table><tbody><tr><td rowspan='2'>{long}</td><td>A</td></tr>\
         <tr><td>B</td></tr></tbody></table>"
    );
    let css = "@page { size: 140pt 90pt; margin: 10pt } \
               table { width:120pt; table-layout:fixed } \
               td { border:0.5pt solid black; font-size:8pt; line-height:10pt }";
    let document = renderer().layout(&html, css).unwrap();
    assert!(document.page_count() > 2);
    let text = document
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .filter_map(|item| match item {
            Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect::<String>();
    for index in 0..120 {
        assert_eq!(text.matches(&format!("TOKEN{index:03}")).count(), 1);
    }
}

#[test]
fn table_footer_repeats_after_page_breaks_without_losing_rows() {
    let rows = (0..18)
        .map(|index| format!("<tr><td>R{index:02}</td></tr>"))
        .collect::<String>();
    let html = format!(
        "<table><thead><tr><th>HEAD</th></tr></thead><tbody>{rows}</tbody>\
         <tfoot><tr><td>FOOT</td></tr></tfoot></table>"
    );
    let css = "@page { size: 140pt 110pt; margin: 10pt } \
               table { width: 120pt } \
               td, th { border: 0.5pt solid black; padding: 1pt; font-size: 8pt; line-height: 10pt }";
    let document = renderer().layout(&html, css).unwrap();
    assert!(document.page_count() > 1);
    let pages = document
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(pages.iter().all(|text| text.contains("HEAD")));
    assert!(pages.iter().all(|text| text.contains("FOOT")));
    let combined = pages.join("|");
    for index in 0..18 {
        assert_eq!(combined.matches(&format!("R{index:02}")).count(), 1);
    }
}

#[test]
fn table_footer_repeat_can_be_disabled_with_row_group_display() {
    let rows = (0..18)
        .map(|index| format!("<tr><td>R{index:02}</td></tr>"))
        .collect::<String>();
    let document = renderer()
        .layout(
            &format!(
                "<table><thead><tr><th>HEAD</th></tr></thead><tbody>{rows}</tbody>\
                 <tfoot><tr><td>FINAL</td></tr></tfoot></table>"
            ),
            "@page { size:140pt 110pt; margin:10pt } table { width:120pt } \
             tfoot { display:table-row-group } \
             td, th { font-size:8pt; line-height:10pt }",
        )
        .unwrap();
    let pages = document
        .pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Item::Text { ch, .. } => Some(*ch),
                    _ => None,
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(pages.len() > 1);
    assert_eq!(
        pages.iter().filter(|text| text.contains("FINAL")).count(),
        1
    );
    assert!(pages.last().unwrap().contains("FINAL"));
}

#[test]
fn collapsed_border_conflicts_choose_the_wider_later_cell_border() {
    let document = renderer()
        .layout(
            "<table><tr><td class='a'>A</td><td class='b'>B</td></tr></table>",
            "table { width:120pt; table-layout:fixed; border-collapse:collapse } \
             td { background:white } .a { border:1pt solid black } .b { border:4pt solid red }",
        )
        .unwrap();
    let red_edges = document.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect {
                x,
                y,
                w,
                h,
                fill: Some(color),
                stroke: None,
            } if *color == Color(1.0, 0.0, 0.0) => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(red_edges
        .iter()
        .any(|(_, _, width, height)| *width >= 3.9 && *height > 10.0));
}

#[test]
fn rowspan_in_repeating_header_is_repeated_as_a_connected_group() {
    let rows = (0..18)
        .map(|index| format!("<tr><td>R{index:02}</td><td>V{index:02}</td></tr>"))
        .collect::<String>();
    let document = renderer()
        .layout(
            &format!(
                "<table><thead><tr><th rowspan='2'>HEAD</th><th>TOP</th></tr>\
                 <tr><th>SUB</th></tr></thead><tbody>{rows}</tbody>\
                 <tfoot><tr><td rowspan='2'>FOOT</td><td>F1</td></tr>\
                 <tr><td>F2</td></tr></tfoot></table>"
            ),
            "@page { size:160pt 110pt; margin:10pt } table { width:140pt; table-layout:fixed } \
             td, th { border:0.5pt solid black; padding:1pt; font-size:8pt; line-height:10pt }",
        )
        .unwrap();
    assert!(document.page_count() > 1);
    for page in &document.pages {
        let text = page
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text { ch, .. } => Some(*ch),
                _ => None,
            })
            .collect::<String>();
        assert!(text.contains("HEAD"));
        assert!(text.contains("TOP"));
        assert!(text.contains("SUB"));
        assert!(text.contains("FOOT"));
        assert!(text.contains("F1"));
        assert!(text.contains("F2"));
    }
}

#[test]
fn nested_table_keeps_its_own_grid_and_content() {
    let document = renderer()
        .layout(
            "<table class='outer'><tr><td>OUT</td><td><table class='inner'>\
             <tr><td>I1</td><td>I2</td></tr><tr><td>I3</td><td>I4</td></tr>\
             </table></td></tr></table>",
            ".outer { width:180pt; table-layout:fixed } \
             .inner { width:80pt; table-layout:fixed; border-spacing:3pt } \
             td { border:0.5pt solid black; padding:1pt } .inner td { background:red }",
        )
        .unwrap();
    let text = document.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect::<String>();
    for expected in ["OUT", "I1", "I2", "I3", "I4"] {
        assert_eq!(text.matches(expected).count(), 1);
    }
    let inner_rects = document.pages[0]
        .items
        .iter()
        .filter(|item| {
            matches!(item, Item::Rect { fill: Some(color), .. } if *color == Color(1.0, 0.0, 0.0))
        })
        .count();
    assert_eq!(inner_rects, 4);
}

#[test]
fn table_column_and_cell_constraints_determine_track_widths() {
    let document = renderer()
        .layout(
            "<table><colgroup><col style='width:25%'><col></colgroup>\
             <tr><td style='max-width:40pt'>A</td><td style='min-width:80pt'>B</td></tr></table>",
            "table { width:160pt; min-width:140pt; max-width:160pt; table-layout:fixed } \
             td { padding:0; background:white }",
        )
        .unwrap();
    assert!((text_x(&document, 'B') - text_x(&document, 'A') - 40.0).abs() < 0.2);
}

#[test]
fn table_wider_than_the_containing_block_is_a_controlled_error() {
    match renderer().layout(
        "<table style='width:120pt'><tr><td>A</td></tr></table>",
        "@page { size:100pt 100pt; margin:10pt }",
    ) {
        Err(error) => assert!(error.to_string().contains("exceeds available width")),
        Ok(_) => panic!("overwide table was silently accepted"),
    }
}

#[test]
fn table_presentational_hints_are_opt_in_and_css_still_wins() {
    let mut renderer = renderer();
    renderer.presentational_hints = true;
    let document = renderer
        .layout(
            "<table width='120' height='60' cellpadding='4' cellspacing='6' border='2'>\
             <tr><td align='right' valign='bottom' height='40'>A</td><td>B</td></tr></table>",
            "table { width:120pt }",
        )
        .unwrap();
    let cells = document.pages[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect { x, y, w, h, .. } => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 2);
    assert!((cells[1].0 - (cells[0].0 + cells[0].2) - 4.5).abs() < 0.2);
    assert!(cells[0].3 >= 40.0);
    assert!(text_x(&document, 'A') > cells[0].0 + cells[0].2 / 2.0);
}
