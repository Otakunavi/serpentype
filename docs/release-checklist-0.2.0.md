# 0.2.0 release checklist

This is the canonical progress tracker for the final 0.2.0 release. A checkbox is closed only when the implementation, mandatory Rust and installed-wheel Python tests, capability entry, and relevant documentation are present. Broad capability groups may remain `partial` while their individual checkboxes close.

Current snapshot after completing the table slice: **38 closed, 34 open**.

## Text and fonts

- [x] Preserve styles for the supported inline HTML elements and mixed font sizes in one line.
- [x] Give `inline-block` an independent inline formatting box for internally wrapped inline text.
- [x] Shape kerning, ligatures, combining sequences, Arabic, Hebrew and mixed bidi text through RustyBuzz when experimental shaping is enabled.
- [ ] Remove the experimental shaping gate after meeting extraction and performance requirements.
- [x] Select fallback fonts per Unicode grapheme cluster and cover Cyrillic, extended Latin and monochrome emoji.
- [x] Support static TTF, CFF OTF, WOFF/WOFF2 and tested variable `wght`/`wdth` faces.
- [x] Return structured missing-glyph data and a typed Python exception.
- [ ] Add source filenames and character-reference positions to missing-glyph diagnostics.
- [x] Support normal/nowrap/pre/pre-wrap, Unicode whitespace and nonbreaking spaces.
- [x] Implement soft-hyphen rendering and `hyphens: manual` in default and shaped text.

## CSS text, units, colors and box model

- [x] Apply justify, text indent, overflow-wrap and word-break before pagination.
- [x] Resolve `em`, `rem`, unitless zero and simple compatible `calc()` expressions.
- [ ] Resolve percentages for margins, padding and general containing-block heights.
- [x] Parse the complete named-color table and opaque RGB/HSL forms deterministically.
- [ ] Export `rgba()`/`hsla()`, slash alpha and `opacity` through PDF graphics state.
- [x] Keep `visibility:hidden` content in layout while suppressing its paint and links.
- [x] Apply `content-box`/`border-box` and fixed/percentage min/max dimensions to paragraph boxes, plus min/max image dimensions.
- [ ] Apply min/max sizing consistently to general block, table, Flex and Grid containers.
- [ ] Implement `overflow:visible|hidden` with predictable clipping.
- [ ] Implement normal-flow margin collapsing, including negative margins.
- [ ] Implement per-side border widths/colors/styles, dashed/dotted borders and `border-radius`.

## Tables and pagination

- [x] Support auto/fixed tables, colspan, body rowspan, cell vertical alignment and repeated headers/optional footers.
- [x] Keep connected rowspan groups together or return a controlled error without losing content.
- [x] Apply one/two-value `border-spacing` to separate table geometry and suppress it for collapsed tables.
- [x] Resolve collapsed-border conflicts deterministically by width and document order within the supported solid-border model.
- [x] Fragment oversized rowspan groups across pages with continued backgrounds and borders without splitting a text line.
- [x] Support connected rowspan groups in repeating headers and footers.
- [x] Lay out nested tables in an independent inner formatting context without flattening or losing content.
- [x] Apply fixed/percentage/min/max sizing to tables, column groups, columns and cells.
- [x] Apply `cellpadding`, `cellspacing`, width/height, align/valign and border presentational hints when explicitly enabled.

## Resources, images and SVG

- [x] Decode PNG, JPEG, WebP and first-frame GIF; apply EXIF orientation and preserve raster alpha.
- [x] Embed eligible JPEG data without recompression.
- [ ] Implement `object-fit`, `object-position` and configurable large-image downsampling.
- [x] Render supported local and data-URI SVG through `resvg` with controlled errors and configurable raster DPI.
- [ ] Preserve supported SVG content as PDF vectors.
- [x] Provide bounded Python `ResourceLoader`, mappings, package resources, callbacks, opt-in HTTP(S) and compatibility `url_fetcher`.
- [ ] Route direct `Renderer` image, SVG and `@font-face` loading through the public resource loader.
- [ ] Propagate cancellation through remote resource operations and finish cache/redirect aggregate-limit tests.

## Positioning, Flex and Grid

- [x] Implement block `position:relative` without moving normal flow.
- [ ] Implement absolute/fixed containing blocks, repeated fixed content, z-index and clipping.
- [ ] Implement translate, scale and rotate transforms.
- [x] Provide the tested row/column Flex subset with basic alignment and gaps.
- [ ] Complete reverse directions, wrapping, grow/shrink/basis, align-self/content and percentage item sizing.
- [x] Provide equal-column Grid tracks and row/column gaps.
- [ ] Complete fixed/percentage/`fr`/auto/minmax/repeat tracks, explicit placement, spans and alignment.
- [ ] Define and test Flex/Grid fragmentation behavior for every supported container mode.

## Paged media and PDF features

- [x] Support break aliases, avoid, orphans/widows, named pages, first/left/right/blank pages and recto/verso breaks.
- [ ] Support combined page selectors and finish the documented break-conflict algorithm.
- [ ] Implement nested counters, running elements and named strings.
- [ ] Implement bleed and crop marks.
- [x] Export external links and tested block/image internal destinations.
- [ ] Export inline anchors, general named destinations and document outlines/bookmarks.
- [x] Export title, author, subject, keywords and document language.
- [ ] Export attachments and configurable creation/modification timestamps.
- [x] Produce byte-identical repeated output on the same tested platform.
- [ ] Verify deterministic object IDs, subsets and compression across every release platform.

## Diagnostics, safety and observability

- [x] Expose severity, code, context fields, occurrences, JSON serialization and deduplication.
- [ ] Add reliable source locations to all parser, CSS, resource and layout diagnostics.
- [ ] Add code/severity filters, allow/deny lists, callbacks/loggers and diagnostic count limits.
- [x] Expose document diagnostics, overflow/missing-glyph/unsupported-feature summaries and render statistics.
- [ ] Make every strict-mode content-loss condition a dedicated typed Python exception.
- [x] Enforce public render limits and poll cancellation during layout and PDF export.
- [ ] Complete adversarial FFI tests proving that no Rust panic crosses into Python.

## Verification and release engineering

- [x] Remove mandatory tests that depend on files outside the repository.
- [x] Run the alpha wheel matrix for manylinux, musllinux, macOS and Windows with CPython 3.10–3.14 ABI3 compatibility tests.
- [ ] Add the remaining required fixtures and visual regression snapshots for every claimed capability.
- [ ] Add property tests and fuzz targets for HTML, CSS, fonts, SVG, raster formats, table fragmentation and PDF serialization.
- [ ] Add multi-process throughput and bounded-cache stress measurements to the benchmark gate.
- [ ] Run the full wheel matrix against the final candidate commit with no mandatory skips.
- [ ] Generate and publish SBOM, build provenance and SHA256 manifests for the final artifacts.
- [ ] Finish changelog, migration notes, capability matrix and final release notes.
- [ ] Create and verify the signed final `v0.2.0` tag.

## Completed after `0.2.0-alpha.1`

- `0d45eea`: `visibility`, paragraph `box-sizing`, paragraph and raster-image min/max dimensions, bidi-aware hidden inline text, Rust regression tests, installed-wheel Python coverage, capability documentation and benchmark verification.
- Current slice: atomic painted inline-blocks, manual soft-hyphen breaks in both text paths, and separate/collapsed table spacing geometry.
- Current table slice: collapsed-border conflict resolution, fragmented rowspan groups, rowspan in repeated sections, nested tables, constrained column sizing and opt-in presentational hints.
