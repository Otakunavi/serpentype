#[test]
fn parses_page_definition_fixture() {
    let source =
        include_str!("../output/benchmark/page_definition_stress/rendered_page_definition.css");
    let sheet = serpentype::css::parse(source, true).unwrap();
    assert_eq!(sheet.named_pages.len(), 8);
    assert_eq!(sheet.first_boxes.len(), 6);
    assert_eq!(sheet.named_pages["appendix_2_ru"].rotation, 90);
    assert_eq!(sheet.named_pages["appendix_2_ru"].width, 841.89);
}

fn font_registry() -> serpentype::font::FontRegistry {
    let fonts = serpentype::font::FontRegistry::new(2_000_000);
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/serpentype/assets/NotoSans-Regular.ttf"
    );
    for family in ["Noto Sans", "Times Newer Roman", "serif"] {
        fonts.register_file(path, family, 400, "normal").unwrap();
    }
    fonts
}

fn page_text(page: &serpentype::layout::Page) -> String {
    page.items
        .iter()
        .filter_map(|item| match item {
            serpentype::layout::Item::Text { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

#[test]
fn named_pages_first_override_and_counters_are_paginated() {
    let css = r#"
        @page { size: A4; margin: 15mm }
        @page :first { @bottom-right { content: "" !important } }
        @page appendix_1_ru {
            @bottom-right { font-size: 8pt; content: "Лист " counter(page) " из " counter(pages) !important }
        }
        @page appendix_2_ru {
            size: landscape !important; page-orientation: rotate-right !important;
            @bottom-right { font-size: 8pt; content: "Лист " counter(page) " из " counter(pages) }
        }
        @media print {
            [data-print="paged"] > .appendix_1_ru { page: appendix_1_ru !important }
            [data-print="paged"] > .appendix_2_ru { page: appendix_2_ru !important }
        }
    "#;
    let html = r#"<article data-print="paged">
      <section class="appendix_1_ru"><p>Первый</p><p style="break-before:page">Второй</p></section>
      <section class="appendix_2_ru"><p>Третий</p></section>
    </article>"#;
    let renderer = serpentype::layout::Renderer::new(font_registry(), ".".into(), true);
    let doc = renderer.layout(html, css).unwrap();
    assert_eq!(doc.page_count(), 3);
    assert!(page_text(&doc.pages[0]).contains("Лист 1 из 3"));
    assert!(page_text(&doc.pages[1]).contains("Лист 2 из 3"));
    assert!(page_text(&doc.pages[2]).contains("Лист 3 из 3"));
    assert_eq!(doc.pages[2].style.rotation, 0);
    assert!(doc.pages[2].style.width > doc.pages[2].style.height);
    assert!(!doc
        .to_pdf()
        .unwrap()
        .windows(10)
        .any(|w| w == b"/Rotate 90"));
    assert_eq!(
        doc.to_pdf().unwrap(),
        renderer.layout(html, css).unwrap().to_pdf().unwrap()
    );
}

#[test]
fn page_orientation_still_rotates_non_landscape_keyword_pages() {
    let renderer = serpentype::layout::Renderer::new(font_registry(), ".".into(), true);
    for size in ["A4", "841pt 595pt"] {
        let css = format!("@page {{ size: {size}; page-orientation: rotate-right }}");
        let doc = renderer.layout("<p>Rotated</p>", &css).unwrap();
        assert_eq!(doc.pages[0].style.rotation, 90);
        assert!(doc
            .to_pdf()
            .unwrap()
            .windows(10)
            .any(|w| w == b"/Rotate 90"));
    }
}

#[test]
fn named_first_page_margin_box_uses_css_cascade() {
    let css = r#"
        @page { size: 150pt 150pt; margin: 20pt;
            @bottom-left { content: "GLOBAL"; color: red; font-size: 8pt }
        }
        @page :first { @bottom-left { content: "FIRST" !important; color: blue } }
        @page named { @bottom-left { content: "NAMED" !important; font-size: 10pt } }
        section { page: named }
    "#;
    let html = "<section><p>one</p><p style='break-before:page'>two</p></section>";
    let renderer = serpentype::layout::Renderer::new(font_registry(), ".".into(), true);
    let doc = renderer.layout(html, css).unwrap();
    assert_eq!(doc.page_count(), 2);
    for page in &doc.pages {
        assert!(page_text(page).contains("NAMED"));
        assert!(!page_text(page).contains("FIRST"));
    }
    let footer = |page: &serpentype::layout::Page| {
        page.items.iter().find_map(|item| match item {
            serpentype::layout::Item::Text {
                ch: 'N',
                color,
                size,
                ..
            } => Some((*color, *size)),
            _ => None,
        })
    };
    assert_eq!(
        footer(&doc.pages[0]),
        Some((serpentype::css::Color(0.0, 0.0, 1.0), 10.0))
    );
    assert_eq!(
        footer(&doc.pages[1]),
        Some((serpentype::css::Color(1.0, 0.0, 0.0), 10.0))
    );

    let normal_named = css.replace("content: \"NAMED\" !important", "content: \"NAMED\"");
    let doc = renderer.layout(html, &normal_named).unwrap();
    assert!(page_text(&doc.pages[0]).contains("FIRST"));
    assert!(page_text(&doc.pages[1]).contains("NAMED"));
}

#[test]
fn all_six_margin_boxes_have_separate_regions_and_first_page_override() {
    let css = r#"
        @page { size: 300pt 300pt; margin: 40pt;
            @top-left { content: "AAAAAAAAAAAAAAAA"; font-size: 8pt }
            @top-center { content: "BBBBBBBBBBBBBBBB"; font-size: 8pt }
            @top-right { content: "CCCCCCCCCCCCCCCC"; font-size: 8pt }
            @bottom-left { content: "DDDDDDDDDDDDDDDD"; font-size: 8pt }
            @bottom-center { content: "EEEEEEEEEEEEEEEE"; font-size: 8pt }
            @bottom-right { content: "FFFFFFFFFFFFFFFF"; font-size: 8pt }
        }
        @page :first {
            @top-left { content: "" } @top-center { content: "" }
            @top-right { content: "" } @bottom-left { content: "" }
            @bottom-center { content: "" } @bottom-right { content: "" }
        }
    "#;
    let renderer = serpentype::layout::Renderer::new(font_registry(), ".".into(), true);
    let html = "<p>first</p><p style='break-before:page'>second</p>";
    let doc = renderer.layout(html, css).unwrap();
    assert_eq!(doc.page_count(), 2);
    assert!(!page_text(&doc.pages[0]).contains('A'));
    assert!(!page_text(&doc.pages[0]).contains('F'));

    let mut counts = [0usize; 6];
    for item in &doc.pages[1].items {
        if let serpentype::layout::Item::Text {
            ch,
            x,
            y,
            font,
            size,
            ..
        } = item
        {
            if let Some(slot) = "ABCDEF".chars().position(|marker| marker == *ch) {
                counts[slot] += 1;
                let column = slot % 3;
                let left = 40.0 + column as f32 * 220.0 / 3.0;
                let right = left + 220.0 / 3.0;
                let advance = font.glyph(*ch).unwrap().1 as f32 * size / font.units_per_em as f32;
                assert!(*x >= left - 0.1, "{ch} starts outside its margin box");
                assert!(
                    *x + advance <= right + 0.1,
                    "{ch} crosses a margin box boundary"
                );
                if slot < 3 {
                    assert!(*y < 40.0, "top margin box is below the margin");
                } else {
                    assert!(*y > 260.0, "bottom margin box is above the margin");
                }
            }
        }
    }
    assert_eq!(counts, [16; 6]);
}

#[test]
fn qr_flex_and_five_column_grid_position_items() {
    let css = r#"
       .qr { display:flex; flex-direction:column; align-items:center; justify-content:center; padding-right:10px }
       .qr img { width:3cm; height:auto; aspect-ratio:1; max-width:100%; max-height:100% }
       .row { display:grid; grid-template-columns:repeat(5, 1fr); align-items:center }
       p { margin:0 }
    "#;
    let html = r#"<div class="qr"><img src="blue.png"></div><div class="row"><p>А</p><p>Б</p><p>В</p><p>Г</p><p>Д</p></div>"#;
    let renderer = serpentype::layout::Renderer::new(
        font_registry(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures").into(),
        true,
    );
    let doc = renderer.layout(html, css).unwrap();
    assert_eq!(doc.page_count(), 1);
    let image = doc.pages[0]
        .items
        .iter()
        .find_map(|item| {
            if let serpentype::layout::Item::Image { x, w, h, .. } = item {
                Some((*x, *w, *h))
            } else {
                None
            }
        })
        .unwrap();
    assert!((image.1 - image.2).abs() < 0.01);
    assert!((image.1 - serpentype::css::length("3cm").unwrap()).abs() < 0.01);
    assert!(image.0 > doc.pages[0].style.margin[3] + 100.0);
    let letters: Vec<_> = doc.pages[0]
        .items
        .iter()
        .filter_map(|item| {
            if let serpentype::layout::Item::Text { ch, x, .. } = item {
                if "АБВГД".contains(*ch) {
                    Some((*ch, *x))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    assert_eq!(letters.len(), 5);
    assert!(letters.windows(2).all(|pair| pair[0].1 < pair[1].1));
}
