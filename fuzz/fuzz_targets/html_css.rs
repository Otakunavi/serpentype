#![no_main]
use libfuzzer_sys::fuzz_target;
use serpentype::font::FontRegistry;
use serpentype::layout::{RenderLimits, Renderer};
use serpentype::{css, html};
use std::path::PathBuf;

fuzz_target!(|data: &[u8]| {
    if data.len() > 32 * 1024 { return; }
    let source = String::from_utf8_lossy(data);
    let split = source.find('\0').unwrap_or(source.len());
    let (markup, stylesheet) = source.split_at(split);
    if let Ok(sheet) = css::parse(stylesheet, false) {
        let _ = html::parse(markup, &sheet, false);
    }
    let fonts = FontRegistry::new(16 * 1024 * 1024);
    let _ = fonts.register_bytes(
        include_bytes!("../../serpentype/assets/NotoSans-Regular.ttf").to_vec(),
        "Noto Sans",
        400,
        "normal",
    );
    let mut renderer = Renderer::new(fonts, PathBuf::from("."), false);
    renderer.limits = RenderLimits {
        max_pages: 8,
        max_input_bytes: 32 * 1024,
        max_nodes: 512,
        max_css_rules: 512,
        max_image_pixels: 1_000_000,
        max_resource_bytes: 1_000_000,
        max_layout_iterations: 20_000,
        restrict_base_dir: true,
    };
    if let Ok(document) = renderer.layout(markup, stylesheet) {
        let _ = document.to_pdf();
    }
});
