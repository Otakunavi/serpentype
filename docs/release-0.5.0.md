# Serpentype 0.5.0

Serpentype 0.5.0 improves text placement against the supplied reference fixtures:

- Text baselines now use the selected font's ascent and descent, with CSS line-box sizing unchanged.
- Center transforms on auto-width blocks now use the block's width and vertical content range, including cases where glyph bounds are narrower than the block.
- Updated PDF visual references to reflect the corrected baseline placement.

## Compatibility verification

The supplied 0.2.1 fixture pack was rendered with four spawned workers using WeasyPrint 64.1 as the reference and Serpentype 0.5.0. The same bundled Noto Sans font was forced in both renderers. All 19 fixtures and 405 pages rendered successfully; page counts and MediaBoxes match. Serpentype reported no diagnostics, overflow, missing glyphs or unsupported features. The 2000-row table renders in 287 pages.

At 72 dpi, the mean absolute grayscale pixel difference over all pages is 3.78 on a 0–255 scale, down from 6.37 for 0.4.0. This comparison is a useful regression signal, not proof of pixel identity. The extracted word sequences differ in order on nine pages, although the normalized word contents match. The reference provides a target for continued compatibility work; the measured results describe the tested fixtures only.

The Rust suite, clippy, Python suite and deterministic PDF check pass locally. The Python suite contains 104 tests and 35 subtests. Cross-platform release wheels are built and verified by CI; only the macOS arm64 development build was verified locally.

Serpentype is AI-generated software, provided as-is and used at the user's own risk. Review and validate generated documents before relying on them.
