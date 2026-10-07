# Serpentype 0.6.0

Serpentype 0.6.0 fixes vertical line drift caused by nested inline elements with smaller font sizes. Inline runs now retain the font metrics and line-height contribution of their inline ancestors, so a short nested run does not shrink a line and shift the content that follows it.

## Compatibility verification

The supplied 0.2.1 fixture pack was rendered using WeasyPrint 64.1 and Serpentype 0.6.0, with the same bundled Noto Sans font forced in both renderers. All 19 fixtures and 405 pages rendered successfully; page counts and MediaBoxes match. Serpentype reported no diagnostics, overflow, missing glyphs or unsupported features. The 2000-row table renders in 287 pages.

At 72 dpi, the mean absolute grayscale pixel difference over all pages is 3.69 on a 0–255 scale, down from 3.78 for 0.5.0 and 6.37 for 0.4.0. This comparison is a regression signal, not proof of pixel identity. Extracted word sequences still differ in order on nine pages, though normalized word contents match. WeasyPrint remains the production default for applications requiring exact rendering or extraction parity.

The Rust suite, clippy, Python suite and deterministic PDF check pass locally. Cross-platform release wheels are built and verified by CI; only the macOS arm64 development build was verified locally.

Serpentype is AI-generated software, provided as-is and used at the user's own risk. Review and validate generated documents before relying on them.
