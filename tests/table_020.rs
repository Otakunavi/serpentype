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
fn oversized_rowspan_group_fails_instead_of_clipping_text() {
    let long = (0..12).map(|_| "line<br>").collect::<String>();
    let html = format!(
        "<table><tbody><tr><td rowspan='2'>{long}</td><td>A</td></tr>\
         <tr><td>B</td></tr></tbody></table>"
    );
    let css = "@page { size: 120pt 100pt; margin: 10pt } \
               td { font-size: 10pt; line-height: 12pt }";
    match renderer().layout(&html, css) {
        Err(error) => assert!(error.to_string().contains("rowspan group")),
        Ok(_) => panic!("oversized rowspan group was clipped"),
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
