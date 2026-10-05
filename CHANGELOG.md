# Changelog

## 0.2.0 — 2026-10-05

- Promote RustyBuzz shaping and Unicode bidi to the default production path while retaining the `experimental_shaping` argument as a compatibility no-op. Missing-glyph errors now locate numeric/common named character references and accept a source filename.
- Complete the planned CSS box slice: percentage margins/padding and block heights, alpha colors and opacity in PDF graphics states, general block/Flex/Grid min/max sizing, overflow clipping, positive/negative margin collapsing, per-side dashed/dotted borders and rounded corners.
- Add atomic `inline-block` boxes with independent sizing, padding, borders, backgrounds, margins and internally wrapped inline text.
- Implement `hyphens: none | manual`; soft hyphens remain invisible unless selected as a line break by the production shaping path.
- Parse `border-collapse` and one/two-value `border-spacing`; apply spacing to separate table tracks and rows while collapsed tables ignore it.
- Complete the 0.2 table scope: deterministic collapsed-border conflicts, line-safe pagination of oversized rowspan groups, rowspan in repeated headers/footers, nested table layout, constrained table/column/cell sizing and opt-in HTML presentational hints.
- Allow footer repetition to be disabled with `tfoot { display: table-row-group }`.
- Add `object-fit`, keyword/percentage/length `object-position` and opt-in `max_image_dpi` raster downsampling.
- Preserve supported SVG as vector PDF Form XObjects, with rasterization limited to embedded raster content and filter subtrees.
- Let direct `Renderer` calls use the bounded public `ResourceLoader` for raster images, SVG and `@font-face`; propagate render cancellation into resource reads and cover cache, redirect and aggregate limits.
- Add block-level absolute/fixed containing blocks, per-page fixed repetition, integer z-order, clipping and translate/scale/rotate transforms. Extend Flex with reverse/wrap/grow/shrink/basis/alignment, Grid with general tracks/placement/spans/alignment, and define atomic Flex-line/Grid-row pagination.
- Complete the planned paged-media/PDF slice: composed named-page pseudo-selectors, documented break precedence, nested counters and margin-box text snapshots, bleed/crop marks, inline and named destinations, hierarchical heading outlines, embedded byte attachments and configurable UTC metadata dates.
- Add Python diagnostic code/severity allow/deny filters, callbacks and count limits; typed `StrictModeError`; and source locations for diagnostics whose HTML/CSS token can be mapped reliably.
- Expose bounded `ResourceLoader` LRU occupancy and add a seeded adversarial corpus, rendered PDF visual references for trim/crop geometry, page sides/blank pages, counters/running/fixed content, table continuation/rowspans/nesting, font fallback/shaping/WOFF2, typography/paint/links, paragraph pagination, layout modes and image resources, Rust panic-boundary tests and cargo-fuzz targets for HTML/CSS, fonts, SVG and PDF serialization.
- Add an independent-process render/cache stress benchmark and release tooling for SPDX SBOMs, provenance metadata, SHA256 manifests and GitHub build attestations.

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
