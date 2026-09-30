use serpentype::font::FontRegistry;
use serpentype::layout::Renderer;
use std::path::PathBuf;

#[test]
fn arabic_uses_contextual_glyphs_when_shaping_is_enabled() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/NotoSansArabic-Regular.ttf");
    let fonts = FontRegistry::new(2_000_000);
    fonts
        .register_file(
            path.to_str().expect("fixture path"),
            "Noto Sans Arabic",
            400,
            "normal",
        )
        .expect("register Arabic fixture");
    let text = "مرحبا";
    let face = fonts
        .resolve(&["Noto Sans Arabic".into()], 400, "normal", 'م')
        .expect("Arabic font");
    let nominal: Vec<_> = text.chars().map(|ch| face.glyph(ch).unwrap().0).collect();
    let shaped = face.shape(text).expect("shape Arabic");
    assert_ne!(
        shaped.iter().map(|glyph| glyph.id).collect::<Vec<_>>(),
        nominal
    );
    assert!(shaped.iter().all(|glyph| glyph.id != 0));

    let mut renderer = Renderer::new(fonts, PathBuf::from("."), false);
    renderer.experimental_shaping = true;
    let html = "<p style=\"font-family:'Noto Sans Arabic'\">مرحبا</p>";
    let document = renderer.layout(html, "").expect("layout Arabic text");
    assert_eq!(document.pages.len(), 1);
    assert!(serpentype::pdf::export(&document)
        .unwrap()
        .starts_with(b"%PDF"));
}

#[test]
fn emoji_zwj_sequence_uses_one_fallback_font_for_the_cluster() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fonts = FontRegistry::new(4_000_000);
    fonts
        .register_file(
            root.join("serpentype/assets/NotoSans-Regular.ttf")
                .to_str()
                .unwrap(),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    fonts
        .register_file(
            root.join("tests/fixtures/NotoEmoji-VF.ttf")
                .to_str()
                .unwrap(),
            "Noto Emoji",
            400,
            "normal",
        )
        .unwrap();

    let cluster = "👨‍👩‍👧";
    let fallback = fonts
        .resolve_cluster(&["Noto Sans".into()], 400, "normal", cluster)
        .unwrap();
    let emoji = fonts
        .resolve_cluster(&["Noto Emoji".into()], 400, "normal", cluster)
        .unwrap();
    assert_eq!(fallback.digest, emoji.digest);

    let mut renderer = Renderer::new(fonts, PathBuf::from("."), true);
    renderer.experimental_shaping = true;
    let document = renderer
        .layout("<p style='font-family:Noto Sans'>Family 👨‍👩‍👧</p>", "")
        .unwrap();
    let emoji_glyphs = document.pages[0]
        .items
        .iter()
        .filter(|item| {
            matches!(item, serpentype::layout::Item::Text { unicode: Some(value), .. } if value.as_ref() == cluster)
        })
        .count();
    assert_eq!(emoji_glyphs, 1);
}

#[test]
fn bidi_isolates_affect_order_without_requiring_or_emitting_glyphs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fonts = FontRegistry::new(4_000_000);
    fonts
        .register_file(
            root.join("serpentype/assets/NotoSans-Regular.ttf")
                .to_str()
                .unwrap(),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    fonts
        .register_file(
            root.join("tests/fixtures/NotoSansArabic-Regular.ttf")
                .to_str()
                .unwrap(),
            "Noto Sans Arabic",
            400,
            "normal",
        )
        .unwrap();
    let mut renderer = Renderer::new(fonts, PathBuf::from("."), true);
    renderer.experimental_shaping = true;
    let document = renderer
        .layout("<p>LTR \u{2067}مرحبا 123\u{2069} END</p>", "")
        .unwrap();
    let actual = document.pages[0].items.iter().find_map(|item| match item {
        serpentype::layout::Item::BeginActualText(value) => Some(value.as_str()),
        _ => None,
    });
    assert_eq!(actual, Some("LTR مرحبا 123 END"));
    assert!(!document.pages[0]
        .items
        .iter()
        .any(|item| { matches!(item, serpentype::layout::Item::Text { glyph: 0, .. }) }));
}

#[test]
fn mixed_hebrew_numbers_and_punctuation_keep_logical_actual_text() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fonts = FontRegistry::new(4_000_000);
    fonts
        .register_file(
            root.join("serpentype/assets/NotoSans-Regular.ttf")
                .to_str()
                .unwrap(),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    fonts
        .register_file(
            root.join("tests/fixtures/NotoSansHebrew-VF.ttf")
                .to_str()
                .unwrap(),
            "Noto Sans Hebrew",
            400,
            "normal",
        )
        .unwrap();
    let mut renderer = Renderer::new(fonts, PathBuf::from("."), true);
    renderer.experimental_shaping = true;
    let logical = "Report (שלום 123) end";
    let document = renderer.layout(&format!("<p>{logical}</p>"), "").unwrap();
    let actual = document.pages[0].items.iter().find_map(|item| match item {
        serpentype::layout::Item::BeginActualText(value) => Some(value.as_str()),
        _ => None,
    });
    assert_eq!(actual, Some(logical));
    assert!(serpentype::pdf::export(&document)
        .unwrap()
        .starts_with(b"%PDF"));
}
