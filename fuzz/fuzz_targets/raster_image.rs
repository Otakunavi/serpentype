#![no_main]
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use libfuzzer_sys::fuzz_target;
use serpentype::font::FontRegistry;
use serpentype::layout::{RenderLimits, Renderer};
use std::path::PathBuf;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() || data.len() > 2 * 1024 * 1024 { return; }
    let encoded = STANDARD.encode(data);
    let html = format!("<img width='32' height='32' src='data:image/png;base64,{encoded}'>");
    let mut renderer = Renderer::new(FontRegistry::new(1024 * 1024), PathBuf::from("."), false);
    renderer.limits = RenderLimits {
        max_input_bytes: 3 * 1024 * 1024,
        max_nodes: 32,
        max_pages: 2,
        max_image_pixels: 1_000_000,
        max_resource_bytes: 2 * 1024 * 1024,
        max_layout_iterations: 10_000,
        restrict_base_dir: true,
        ..RenderLimits::default()
    };
    if let Ok(document) = renderer.layout(&html, "@page { size: 120pt 120pt; margin: 5pt }") {
        let _ = document.to_pdf();
    }
});
