# Serpentype 0.2.0-alpha.1

This alpha publishes the current 0.2.0 development work for integration testing. It preserves the existing Python entry points while expanding the controlled HTML/CSS subset.

## Highlights

- Styled inline fragments, links, superscript/subscript and mixed line heights.
- Opt-in RustyBuzz shaping, grapheme-level fallback, Arabic/Hebrew bidi controls and logical PDF `/ActualText`.
- TTF, CFF OTF, WOFF/WOFF2 and tested variable `wght`/`wdth` fonts.
- Fixed/auto tables with `colspan`, body `rowspan`, repeated `thead`/`tfoot` and cell vertical alignment.
- PNG, JPEG, WebP, first-frame GIF, EXIF orientation, raster alpha and `resvg` SVG fallback.
- Bounded compatibility `ResourceLoader`, render limits, cancellation, diagnostics and render statistics.
- Internal/external PDF links, document metadata and deterministic local PDF regression coverage.
- Partial relative positioning, Flex/Grid gaps and paged-media page selectors/breaks.

## Verification

- 43 Rust integration tests.
- 61 Python tests against an installed ABI3 wheel.
- Rust formatting and clippy with warnings denied.
- Local macOS arm64 / CPython 3.13 wheel installation and PDF export.
- Performance remained inside the configured layout, export and RSS gates.

## Alpha limitations

Shaping remains opt-in and experimental. Absolute/fixed positioning, complete Flex/Grid, running elements, named strings, vector SVG PDF output, collapsed table borders, nested tables, cross-page rowspan fragmentation, object fitting, alpha CSS colors, bookmarks, attachments, cross-platform deterministic output and comprehensive fuzzing remain incomplete. The direct Rust-backed `Renderer` does not yet use the Python `ResourceLoader`.

Use `serpentype.capabilities()` and `Renderer.supports()` to inspect the machine-readable support levels. See [the full capability matrix](capabilities.md) and [migration notes](migration-0.2.0.md).
