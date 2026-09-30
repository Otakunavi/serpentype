# Changelog

## 0.2.0-alpha.1 — 2026-09-30

- Keep the existing `Renderer`, `PreparedDocument`, `FontRegistry`, `HTML`, `CSS`, `Document` and `FontConfiguration` call forms.
- Improve inline text handling for styled fragments, `u`, `a`, `sup` and `sub`; add external PDF link annotations.
- Select one registered fallback face for each Unicode grapheme cluster, including tested monochrome emoji ZWJ sequences. Accept CFF-outline OTF, WOFF and WOFF2 fonts; instance variable `wght` and `wdth` axes for PDF export; support CSS `font-stretch`; add opt-in synthetic bold and italic. Missing glyphs raise structured `MissingGlyphError` with string-HTML line/column while retaining `ValueError` compatibility.
- Add opt-in RustyBuzz shaping with Latin ligatures and kerning, combining marks, Arabic and Hebrew glyph shaping, mixed LTR/RTL visual ordering, numbers and punctuation, and explicit bidi embedding/isolate controls. Controls affect Unicode bidi ordering without requiring or emitting font glyphs. Mixed lines include exact visible logical PDF `/ActualText`; pypdf and pdfminer currently ignore it and may omit a space adjacent to RTL text.
- Extend table placement with body `rowspan`, cell `vertical-align`, repeated `tfoot`, column occupancy validation and page-safe rowspan groups. Oversized span groups and spans crossing row-group boundaries fail with controlled errors instead of clipping content.
- Add basic justify, text indent, whitespace modes, contextual lengths, named/RGB/HSL colors and percentage widths.
- Decode WebP and the first GIF frame alongside PNG and JPEG; preserve raster alpha with a PDF soft mask, embed unrotated RGB JPEG bytes directly, decode grayscale JPEG into the PDF RGB space, and apply EXIF orientation when needed.
- Replace the compatibility adapter's restricted Pillow SVG renderer with `resvg` for local and base64 SVG images. Add configurable raster DPI, registered-font SVG text and controlled errors for unavailable SVG glyphs or external image references.
- Export internal links to paragraph, heading and image IDs as PDF destinations, including forward references.
- Add opt-in `base_dir` confinement for local images and `@font-face` resources, including symlink checks.
- Accept base64 raster `data:` image URLs with media-type validation and resource byte limits.
- Export document title, author, subject, keywords and language from HTML metadata.
- Add structured deduplicated diagnostics, a machine readable capability matrix, render limits, cancellation and render statistics.
- Add partial `colspan` and fixed table layout, page header/data keep-together handling, and `orphans`/`widows` paragraph pagination.
- Add `position: relative` for block painting, `gap` for the existing Flex/Grid subset, left/right/recto/verso page breaks and `@page :first/:left/:right/:blank` styles.
- Add a bounded Python `ResourceLoader` and `url_fetcher` support for compatibility `HTML`/`CSS` objects.
- Make the page-definition tests independent of files outside the repository and add ABI3 wheel CI targets.

This is an alpha prerelease. The final 0.2.0 release remains blocked by the unsupported and partial entries in [the capability matrix](docs/capabilities.md).
