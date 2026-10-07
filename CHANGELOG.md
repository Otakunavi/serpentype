# Changelog

## Unreleased

- Preserve ancestor inline font metrics and line-height when nested inline elements change font size; this keeps following lines from shifting vertically.
- Add a regression test for nested inline font-size changes across explicit line breaks.
- Re-run the supplied 19-fixture compatibility pack: all 405 pages render with matching page counts and MediaBoxes; mean grayscale pixel error improves from 3.78 to 3.69/255 compared with WeasyPrint 64.1.
- This work remains unreleased: extracted word order differs on nine pages and visual output is not pixel-identical.

## 0.5.0 — 2026-10-07

- Position text baselines from the selected font's actual ascent and descent while preserving the CSS line-box height.
- Compute centered transforms for auto-width blocks from the block's border-box width and vertical content range instead of the painted glyph bounds.
- Verify the supplied 19-fixture pack against WeasyPrint 64.1: all 405 pages render, with matching page counts and MediaBoxes and no Serpentype diagnostics, overflow, missing glyphs or unsupported-feature reports.
- Record the remaining compatibility limits: the same words are extracted in a different order on nine pages, and full pixel-identical rendering is not established.

## 0.4.0 — 2026-10-07

- Fix wrapping around trailing collapsible spaces, inline wrappers ending in block content, and flow positioning after a fragmented block.
- Use intrinsic text widths for auto-sized inline flex items and remove a collapsed-table-cell text-width adjustment that distorted layout.
- Apply HTML default middle alignment to table data cells without leaking non-inherited vertical alignment into text styles.
- Expand compatibility verification to confirm matching page counts, page boxes and normalized text content across all 405 pages in the 19-fixture pack against WeasyPrint 64.1 with the bundled font forced in both engines.
- Preserve WeasyPrint as the production default: text extraction order differs on nine pages, and pixel-perfect equivalence across all fixtures has not been established.

## 0.3.0 — 2026-10-07

- Preserve inline and inline-block display on block HTML tags, empty inline-block placeholders and empty `<br>` line boxes instead of dropping them during flow splitting.
- Compute inherited unitless `line-height` against each element's font size, including inline runs, and center font metrics within the resulting line box.
- Account for half-width borders in collapsed table-cell content and fragment geometry; the supplied 19-fixture pack now matches WeasyPrint 64.1 page counts with the same bundled font.
- Recompute auto-width block frames when page size changes, so tables inside nested full-width containers use the named page's landscape content width.
- Promote block descendants out of nested inline wrappers during flow layout; production-shaped tables inside spans now retain column, row and border layout.
- Parse the common `font` shorthand in page-margin boxes and avoid warnings for identity `transform-origin`, `filter: none`/`blur(0)`, `outline: none`, and inert empty positioned spans.
- Add balanced `column-count` layout for inline-only block content and document its fragmentation boundary.
- Raise the default-limit long-table fixture result from a limit error to a successful 287-page document; the named landscape layout now fits within the 500-page default.

## 0.2.1 — 2026-10-06

- Pass optional `RenderLimits` through the WeasyPrint-shaped `HTML.render()` and `HTML.write_pdf()` methods, so compatibility callers can configure the page and resource bounds.
- Record the October 2026 production-template comparison: Serpentype is not a drop-in WeasyPrint renderer; landscape table sizing and malformed nested-table markup need fixture-based review, and long-document page limits must be selected for the workload.
- Keep the WeasyPrint backend as the production default until the affected templates pass visual golden comparisons.

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
