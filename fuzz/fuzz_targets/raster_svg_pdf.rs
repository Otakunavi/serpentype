#![no_main]
use libfuzzer_sys::fuzz_target;
use serpentype::layout::{RenderLimits, Renderer};
use serpentype::font::FontRegistry;
use std::path::PathBuf;

fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 { return; }
    let input = String::from_utf8_lossy(data);
    let escaped = input.replace('&', "&amp;").replace('"', "&quot;");
    let html = format!("<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'><text>{}</text></svg>", escaped);
    let mut renderer = Renderer::new(FontRegistry::new(1024 * 1024), PathBuf::from("."), false);
    renderer.limits = RenderLimits { max_input_bytes: 32 * 1024, max_nodes: 256, max_pages: 4, max_resource_bytes: 1024 * 1024, max_layout_iterations: 10_000, restrict_base_dir: true, ..RenderLimits::default() };
    if let Ok(document) = renderer.layout(&html, "@page { size: 120pt 120pt; margin: 5pt }") {
        let _ = document.to_pdf();
    }
});
