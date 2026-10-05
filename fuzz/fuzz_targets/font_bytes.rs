#![no_main]
use libfuzzer_sys::fuzz_target;
use serpentype::font::FontRegistry;

fuzz_target!(|data: &[u8]| {
    if data.len() > 8 * 1024 * 1024 { return; }
    let registry = FontRegistry::new(8 * 1024 * 1024);
    let _ = registry.register_bytes(data.to_vec(), "Fuzz", 400, "normal");
});
