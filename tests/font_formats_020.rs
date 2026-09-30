use serpentype::font::FontRegistry;
use serpentype::layout::Renderer;
use std::io::Read;
use std::path::PathBuf;

#[test]
fn cff_otf_and_web_font_containers_render_and_export() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for extension in ["otf", "otf.woff", "otf.woff2"] {
        let registry = FontRegistry::new(2_000_000);
        let path = root.join(format!("SourceSans3-Regular.{extension}"));
        registry
            .register_file(path.to_str().unwrap(), "Source Sans 3", 400, "normal")
            .unwrap();
        let renderer = Renderer::new(registry, ".".into(), true);
        let doc = renderer
            .layout("<p style='font-family:Source Sans 3'>CFF Àé Привет</p>", "")
            .unwrap();
        let pdf = serpentype::pdf::export(&doc).unwrap();
        assert!(pdf
            .windows(b"/CIDFontType0".len())
            .any(|v| v == b"/CIDFontType0"));
        assert!(pdf.windows(b"/FontFile3".len()).any(|v| v == b"/FontFile3"));
    }
}

#[test]
fn variable_weight_changes_embedded_font_instance() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let registry = FontRegistry::new(2_000_000);
    registry
        .register_file(
            root.join("SourceSans3VF-Upright.ttf").to_str().unwrap(),
            "Source Sans VF",
            400,
            "normal",
        )
        .unwrap();
    let normal = registry
        .resolve(&["Source Sans VF".into()], 400, "normal", 'A')
        .unwrap();
    let bold = registry
        .resolve(&["Source Sans VF".into()], 700, "normal", 'A')
        .unwrap();
    assert_ne!(normal.digest, bold.digest);
    let renderer = Renderer::new(registry, ".".into(), true);
    let normal = renderer
        .layout("<p style='font-family:Source Sans VF'>Variable</p>", "")
        .unwrap();
    let bold = renderer
        .layout(
            "<p style='font-family:Source Sans VF;font-weight:700'>Variable</p>",
            "",
        )
        .unwrap();
    assert_ne!(
        serpentype::pdf::export(&normal).unwrap(),
        serpentype::pdf::export(&bold).unwrap()
    );
}

#[test]
fn font_stretch_uses_variable_width_axis() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let registry = FontRegistry::new(4_000_000);
    registry
        .register_file(
            root.join("RobotoFlex-VF.ttf").to_str().unwrap(),
            "Roboto Flex",
            400,
            "normal",
        )
        .unwrap();
    let condensed = registry
        .resolve_with_stretch(&["Roboto Flex".into()], 400, "normal", 75.0, 'M')
        .unwrap();
    let expanded = registry
        .resolve_with_stretch(&["Roboto Flex".into()], 400, "normal", 125.0, 'M')
        .unwrap();
    assert_ne!(condensed.digest, expanded.digest);
    assert!(condensed.glyph('M').unwrap().1 < expanded.glyph('M').unwrap().1);

    let renderer = Renderer::new(registry, ".".into(), true);
    let condensed = renderer
        .layout(
            "<p style='font-family:Roboto Flex;font-stretch:condensed'>MMMM</p>",
            "",
        )
        .unwrap();
    let expanded = renderer
        .layout(
            "<p style='font-family:Roboto Flex;font-stretch:expanded'>MMMM</p>",
            "",
        )
        .unwrap();
    let condensed_pdf = serpentype::pdf::export(&condensed).unwrap();
    assert_eq!(condensed_pdf, serpentype::pdf::export(&condensed).unwrap());
    assert_ne!(condensed_pdf, serpentype::pdf::export(&expanded).unwrap());
}

#[test]
fn synthetic_faces_are_explicit_and_visible_in_pdf_text_state() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = FontRegistry::new(2_000_000);
    registry
        .register_file(
            root.join("serpentype/assets/NotoSans-Regular.ttf")
                .to_str()
                .unwrap(),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    let mut renderer = Renderer::new(registry, ".".into(), true);
    renderer.synthetic_bold = true;
    renderer.synthetic_italic = true;
    let doc = renderer
        .layout(
            "<p style='font-weight:700;font-style:italic'>Synthetic</p>",
            "",
        )
        .unwrap();
    let font = doc.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            serpentype::layout::Item::Text { font, .. } => Some(font),
            _ => None,
        })
        .unwrap();
    assert!(font.synthetic_bold);
    assert!(font.synthetic_italic);
    let pdf = serpentype::pdf::export(&doc).unwrap();
    let mut text_ops = String::new();
    let mut cursor = 0;
    while let Some(start) = pdf[cursor..]
        .windows(b"stream\n".len())
        .position(|part| part == b"stream\n")
    {
        let data_start = cursor + start + b"stream\n".len();
        let Some(end) = pdf[data_start..]
            .windows(b"\nendstream".len())
            .position(|part| part == b"\nendstream")
        else {
            break;
        };
        let mut decoder = flate2::read::ZlibDecoder::new(&pdf[data_start..data_start + end]);
        let mut decoded = String::new();
        if decoder.read_to_string(&mut decoded).is_ok() {
            text_ops.push_str(&decoded);
        }
        cursor = data_start + end + b"\nendstream".len();
    }
    assert!(text_ops.contains("2 Tr"));
    assert!(text_ops.contains("0.220"));
}

#[test]
fn nearest_italic_face_prefers_oblique_over_upright() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = FontRegistry::new(2_000_000);
    registry
        .register_file(
            root.join("serpentype/assets/NotoSans-Regular.ttf")
                .to_str()
                .unwrap(),
            "Mixed",
            400,
            "normal",
        )
        .unwrap();
    registry
        .register_file(
            root.join("tests/fixtures/SourceSans3-Regular.otf")
                .to_str()
                .unwrap(),
            "Mixed",
            400,
            "oblique",
        )
        .unwrap();
    let selected = registry
        .resolve(&["Mixed".into()], 400, "italic", 'A')
        .unwrap();
    let oblique = registry
        .resolve(&["Mixed".into()], 400, "oblique", 'A')
        .unwrap();
    assert_eq!(selected.digest, oblique.digest);
}

#[test]
fn monochrome_emoji_uses_registered_glyph_fallback() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = FontRegistry::new(4_000_000);
    registry
        .register_file(
            root.join("serpentype/assets/NotoSans-Regular.ttf")
                .to_str()
                .unwrap(),
            "Noto Sans",
            400,
            "normal",
        )
        .unwrap();
    registry
        .register_file(
            root.join("tests/fixtures/NotoEmoji-VF.ttf")
                .to_str()
                .unwrap(),
            "Noto Emoji",
            400,
            "normal",
        )
        .unwrap();
    let fallback = registry
        .resolve(&["Noto Sans".into()], 400, "normal", '😀')
        .unwrap();
    let emoji = registry
        .resolve(&["Noto Emoji".into()], 400, "normal", '😀')
        .unwrap();
    assert_eq!(fallback.digest, emoji.digest);
    let renderer = Renderer::new(registry, ".".into(), true);
    let doc = renderer
        .layout("<p style='font-family:Noto Sans'>Hello 😀</p>", "")
        .unwrap();
    let pdf = serpentype::pdf::export(&doc).unwrap();
    assert!(!pdf.is_empty());
}

#[test]
fn malformed_web_font_returns_controlled_error() {
    let registry = FontRegistry::new(2_000_000);
    let mut woff = b"wOFF".to_vec();
    woff.resize(44, 0);
    woff[16..20].copy_from_slice(&(100_000_001_u32).to_be_bytes());
    let error = registry
        .register_bytes(woff, "Malformed", 400, "normal")
        .unwrap_err();
    assert!(error.to_string().contains("invalid decoded size"));
}
