# 0.2.0 release checklist

This is the canonical progress tracker for the final 0.2.0 release. A checkbox is closed only when the implementation, mandatory Rust and installed-wheel Python tests, capability entry, and relevant documentation are present. Broad capability groups may remain `partial` while their individual checkboxes close.

Current snapshot: **69 closed, 4 open**.

## Text and fonts

- [x] Preserve styles for the supported inline HTML elements and mixed font sizes in one line.
- [x] Give `inline-block` an independent inline formatting box for internally wrapped inline text.
- [x] Shape kerning, ligatures, combining sequences, Arabic, Hebrew and mixed bidi text through RustyBuzz.
- [x] Remove the experimental shaping gate after meeting extraction and performance requirements.
- [x] Select fallback fonts per Unicode grapheme cluster and cover Cyrillic, extended Latin and monochrome emoji.
- [x] Support static TTF, CFF OTF, WOFF/WOFF2 and tested variable `wght`/`wdth` faces.
- [x] Return structured missing-glyph data and a typed Python exception.
- [x] Add source filenames and character-reference positions to missing-glyph diagnostics.
- [x] Support normal/nowrap/pre/pre-wrap, Unicode whitespace and nonbreaking spaces.
- [x] Implement soft-hyphen rendering and `hyphens: manual` in default and shaped text.

## CSS text, units, colors and box model

- [x] Apply justify, text indent, overflow-wrap and word-break before pagination.
- [x] Resolve `em`, `rem`, unitless zero and simple compatible `calc()` expressions.
- [x] Resolve percentages for margins, padding and general containing-block heights.
- [x] Parse the complete named-color table and opaque RGB/HSL forms deterministically.
- [x] Export `rgba()`/`hsla()`, slash alpha and `opacity` through PDF graphics state.
- [x] Keep `visibility:hidden` content in layout while suppressing its paint and links.
- [x] Apply `content-box`/`border-box` and fixed/percentage min/max dimensions to paragraph boxes, plus min/max image dimensions.
- [x] Apply min/max sizing consistently to general block, table, Flex and Grid containers.
- [x] Implement `overflow:visible|hidden` with predictable clipping.
- [x] Implement normal-flow margin collapsing, including negative margins.
- [x] Implement per-side border widths/colors/styles, dashed/dotted borders and `border-radius`.

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
- [x] Implement `object-fit`, `object-position` and configurable large-image downsampling.
- [x] Render supported local and data-URI SVG through `resvg` with controlled errors and configurable raster DPI.
- [x] Preserve supported SVG content as PDF vectors.
- [x] Provide bounded Python `ResourceLoader`, mappings, package resources, callbacks, opt-in HTTP(S) and compatibility `url_fetcher`.
- [x] Route direct `Renderer` image, SVG and `@font-face` loading through the public resource loader.
- [x] Propagate cancellation through remote resource operations and finish cache/redirect aggregate-limit tests.

## Positioning, Flex and Grid

- [x] Implement block `position:relative` without moving normal flow.
- [x] Implement absolute/fixed containing blocks, repeated fixed content, z-index and clipping.
- [x] Implement translate, scale and rotate transforms.
- [x] Provide the tested row/column Flex subset with basic alignment and gaps.
- [x] Complete reverse directions, wrapping, grow/shrink/basis, align-self/content and percentage item sizing.
- [x] Provide equal-column Grid tracks and row/column gaps.
- [x] Complete fixed/percentage/`fr`/auto/minmax/repeat tracks, explicit placement, spans and alignment.
- [x] Define and test Flex/Grid fragmentation behavior for every supported container mode.

## Paged media and PDF features

- [x] Support break aliases, avoid, orphans/widows, named pages, first/left/right/blank pages and recto/verso breaks.
- [x] Verify combined named-page selectors with `:first`, `:left`, `:right` and `:blank`.
- [x] Finish the documented break-conflict algorithm for the supported break properties.
- [x] Implement nested counters, text-only running elements and named strings in margin boxes.
- [x] Implement bleed page boxes and crop marks.
- [x] Export external links and tested block/image internal destinations.
- [x] Export inline anchors, general named destinations and hierarchical heading outlines/bookmarks.
- [x] Export title, author, subject, keywords and document language.
- [x] Export attachments and configurable creation/modification timestamps.
- [x] Produce byte-identical repeated output on the same tested platform.
- [ ] Verify deterministic object IDs, subsets and compression across every release platform (native Linux/macOS/Windows SHA256 gate added; final full wheel matrix still required).

## Diagnostics, safety and observability

- [x] Expose severity, code, context fields, occurrences, JSON serialization and deduplication.
- [x] Add reliable source locations to parser, CSS, resource and layout diagnostics (HTML/CSS tokens, malformed input, strict errors, resource limits and layout/missing-glyph failures have 1-based source/line/column; input-wide limits and cancellation have no unique source token).
- [x] Add code/severity filters, allow/deny lists, callbacks/loggers and diagnostic count limits.
- [x] Expose document diagnostics, overflow/missing-glyph/unsupported-feature summaries and render statistics.
- [x] Make every strict-mode content-loss condition a dedicated typed Python exception.
- [x] Enforce public render limits and poll cancellation during layout and PDF export.
- [x] Complete adversarial FFI tests proving that no Rust panic crosses into Python.

## Verification and release engineering

- [x] Remove mandatory tests that depend on files outside the repository.
- [x] Run the alpha wheel matrix for manylinux, musllinux, macOS and Windows with CPython 3.10–3.14 ABI3 compatibility tests.
- [x] Add regression fixtures for every rendered capability: checked visual references cover trim/crop geometry, first/left/right/blank pages, counters/running/fixed content, table continuation/rowspans/nesting, font fallback/shaping/WOFF2, typography/paint/links, paragraph pagination, Flex/Grid/positioning/clipping, and raster/SVG. PDF metadata, attachments, outlines and annotations are covered by structural PDF assertions rather than pixel snapshots.
- [x] Add property tests and fuzz targets for HTML, CSS, fonts, SVG, raster formats, table fragmentation and PDF serialization.
- [x] Add multi-process throughput and bounded-cache stress measurements to the benchmark gate.
- [ ] Run the full wheel matrix against the final candidate commit with no mandatory skips.
- [ ] Generate and publish SBOM, build provenance and SHA256 manifests for the final artifacts.
- [x] Finish changelog, migration notes, capability matrix and final release notes.
- [ ] Create and verify the signed final `v0.2.0` tag.

## Completed after `0.2.0-alpha.1`

- `0d45eea`: `visibility`, paragraph `box-sizing`, paragraph and raster-image min/max dimensions, bidi-aware hidden inline text, Rust regression tests, installed-wheel Python coverage, capability documentation and benchmark verification.
- Current slice: atomic painted inline-blocks, manual soft-hyphen breaks in both text paths, and separate/collapsed table spacing geometry.
- Current table slice: collapsed-border conflict resolution, fragmented rowspan groups, rowspan in repeated sections, nested tables, constrained column sizing and opt-in presentational hints.
- Current resource slice: image fitting/positioning, opt-in raster downsampling, vector SVG Form XObjects, direct `Renderer` loader integration and aggregate/cancellation loader coverage.
- Current positioning/layout slice: block absolute/fixed layers and transforms, extended Flex/Grid sizing and alignment, explicit Grid placement/spans, and tested fragmentation boundaries.
- Current paged-media/PDF slice: combined page selectors and break precedence, nested counters/text running content/named strings, bleed/crop marks, general and inline destinations with hierarchical heading outlines, PDF attachments/UTC timestamps, and a native-platform deterministic-PDF SHA256 gate. Full wheel-matrix verification remains open until CI passes.
- Current release-hardening slice: typed strict-mode errors and diagnostic policies; mapped HTML/CSS source positions; bounded resource-cache statistics; property/fuzz harnesses; fourteen rendered-page snapshots across trim/crop geometry, page sides/blank pages, counters/running/fixed content, table continuation/rowspans/nested tables, font fallback/shaping/WOFF2, typography/paint/links, paragraph pagination, layout modes and image resources; measured independent-process/cache stress; SPDX/provenance/checksum generation and release attestation workflow. Publishing and signing require the final tagged candidate and valid GitHub credentials.
- Current diagnostic-location slice: source offsets flow through parsed HTML nodes into layout errors; CSS diagnostics distinguish external, embedded and inline declarations; resource-loader exceptions, local @font-face failures, strict-mode errors and margin-box missing glyphs include source/line/column metadata. Regression coverage also guards against false offsets from HTML comments and raw-text elements.
