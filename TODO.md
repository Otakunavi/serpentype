# TODO

## Align the Serpentype renderer with the reference

WeasyPrint 64.1 is the canonical reference implementation. Detailed
measurements, line boxes, resource hashes, and Poppler font/image inventories
are in
[`docs/weasyprint_similarity.md`](docs/weasyprint_similarity.md) and
[`output/compatibility/weasyprint-comparison.json`](output/compatibility/weasyprint-comparison.json).
WeasyPrint remains outside the production API.

### Observed mismatches

- [x] **Page selectors: match page-specific content and line placement.** Fixed
  auto-height `bottom: 0` positioning, right-aligned auto-width sizing, and
  center margin-box alignment. The fixture now has exact normalized text and
  page counts; its documented max line-box delta is 3.77 pt.
- [x] **Typography: match pagination and manual hyphenation.** Normal line
  height now uses the bundled Noto Sans metrics, so the fixture paginates to
  two pages. Selected manual soft hyphens paint U+2010; comparison treats
  U+00AD as invisible and preserves raw extracted text.
- [x] **Layout: match rotated-content line wrapping.** Inline-block transforms
  now wrap both the box and its contents; Poppler extracts the same normalized
  word and line sequences as the reference.
- [x] **Page boxes: match `bleed` and crop-mark dimensions.** Bleed expands the
  MediaBox by the declared amount only. The release fixture now matches the
  reference page dimensions.
- [x] **Reduce residual line-position drift after text and pagination match.**
  The equal-fixture mean fell from 7.17 pt in the 0.7.0/WeasyPrint 64.1 baseline
  to 0.64 pt. Per-line exceptions remain documented for transformed layout
  (max 10.24 pt), auto-sized nested tables (max 11.99 pt), and page selectors
  (max 3.77 pt); the report retains each line box so these values remain
  reviewable.
- [x] **Keep production templates renderable when an inline-block has no
  usable content width.** Added the supplied notice and appendix HTML as
  fixtures. Serpentype now emits an `inline-block-width` warning instead of
  failing PDF generation; both templates render in Serpentype and WeasyPrint
  to four pages each, with matching page dimensions.
- [ ] **Restore content and improve text-flow parity in the production
  templates.** Serpentype currently omits the QR placeholder's inline-block
  content after warning. With the shared Noto Sans font, word sequence
  similarity is 96.51% for the notice and 99.30% for the appendix; normalized
  line sequence similarity is 57.91% and 79.88%. The page counts match, but
  line wrapping and some extracted text still differ. See the per-line details
  in the JSON report.

### Comparison coverage and reproducibility

- [x] **Rerun with the pinned reference version.** The report uses the pinned
  WeasyPrint 64.1 and records the actual runtime and input hashes.
- [x] **Measure visual differences beyond text extraction.** The report adds
  Poppler `pdffonts` and `pdfimages -list` inventories, line boxes, local
  resource hashes, raster page metrics, and cropped difference regions. Raster
  comparison does not isolate colors, vector paths, glyph outlines, or paint
  order; full visual parity is not claimed.
- [x] **Cover each repaired mismatch and hash its inputs.** The fourteen-fixture
  corpus represents selectors, fixed content, transforms, typography, manual
  hyphens, tables, bleed/crop boxes, the supplied production templates, and
  Russian and Kazakh notices.
  Added Rust regressions cover auto-height fixed positioning and transformed
  inline boxes. The report hashes every HTML/CSS fixture and each referenced
  local resource.

### Completion criteria

- [x] All fourteen fixtures produce matching page counts; the report covers
  35 pages on each side. The four supplied production templates have matching
  page dimensions (four or five pages each).
- [x] The original ten-fixture canonical corpus retains exact extracted word
  and normalized line sequences; U+00AD/U+2010 handling is recorded in the
  report.
- [ ] The two production templates have exact extracted word and normalized
  line sequences. Current similarities and line differences are recorded in
  the report; continue after resolving the inline-block content-width issue.
- [x] Centered line-box deltas are recorded for every text-identical line.
  Remaining fixture-specific values above 1 pt p95 or 2 pt max are documented
  with their affected fixtures and remain visible in the JSON.
- [x] Raster comparison records page-level metrics and crops for changed
  regions; 14 of 35 pages meet the documented tolerance. Vector SVG structure
  remains unmeasured, and no visual parity claim is made.
- [x] The pinned WeasyPrint baseline and repeat-run extracted-layout hashes
  reproduce on the documented environment.
