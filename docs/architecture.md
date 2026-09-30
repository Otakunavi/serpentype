# Architecture and invariants

- `html.rs` parses HTML5 syntax into an owned visible tree and reports unsupported tags and table spans.
- `css.rs` parses the documented print CSS subset, including nested `@page` and `@media print`, selectors and `!important`, and inherits text properties.
- `font.rs` imports explicit TTF-outline fonts. SHA-256 of the bytes is the cache key; the supported face index is always zero. Parsed metadata and glyph metrics are held in `Arc<FontData>`. Shared cache bookkeeping is locked briefly and is bounded by `cache_bytes`. Layouts receive isolated family registrations so concurrent `@font-face` declarations cannot change another layout's font lookup. The cache itself is shared. Eviction/clear only drops cache ownership; active documents and registrations may use memory beyond that limit. No shaping cache is used.
- `layout.rs` loads local images, caches decoded RGB by content hash up to 32 MiB, computes line and cell geometry, and builds positioned page display lists. It switches page geometry on named-page transitions and adds margin box content after the final page count is known. Images are re-read at layout to detect changes at the same path. The prepared document retains RGB and font bytes.
- `pdf.rs` serializes display lists directly as PDF text, vector rectangles, and image XObjects. It subsets each used TrueType font, remaps the PDF glyph IDs, widths and ToUnicode CMap, and embeds the subset for text extraction. It never calls the layout engine or reads external files. PDF object IDs and used glyph maps are document-local.
- `python.rs` exposes the Rust core via PyO3 and releases the GIL for layout and export.

All geometry is measured in PDF points (`pt`). CSS `1px = 0.75pt`, `1mm = 72/25.4pt`, and `1in = 72pt`. Positions in the display list use a top-left origin; PDF export converts them to PDF's bottom-left origin.

## Pagination rules

Paragraph lines are wrapped to the available content width and placed as they are paginated. A line, padding and border must fit inside the full page content rectangle or layout fails. `break-inside: avoid` moves a whole block only if it fits on a blank page; for a taller block it is relaxed and the block is split by lines. A pending `break-after: page` takes effect before the next content block, so it does not create a trailing blank page.

Table widths use explicit cell widths where present and otherwise the longest word in each column as a minimum estimate; desired widths are scaled to the table width. A placement pass reserves occupied columns for `rowspan` and validates that spans remain inside their row group. Connected body rowspan groups move together when they fit on a page. A group taller than the area between repeated headers and footers returns a controlled error instead of clipping content. Ordinary oversized rows still split by whole text lines. `<thead>` and `<tfoot>` repeat around every body fragment, and pagination reserves their height before placing data. Every loop iteration either places at least one line or returns an error.

The display list is logically immutable after layout. `page_count` is `pages.len()` and PDF export creates one page object per display page.
