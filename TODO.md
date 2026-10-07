# TODO

## WeasyPrint similarity for the Serpentype renderer

The canonical baseline is WeasyPrint 64.1. Detailed measurements, line boxes,
resource hashes, and Poppler font/image inventories are in
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

### Comparison coverage and reproducibility

- [x] **Rerun with the pinned reference version.** The report uses the pinned
  WeasyPrint 64.1 and records the actual runtime and input hashes.
- [x] **Check visual objects absent from text extraction.** The report adds
  Poppler `pdffonts` and `pdfimages -list` inventories, and records line boxes
  and local resource hashes. For colors, backgrounds, paths, glyph outlines,
  image placement, and border appearance, the report defines normalized
  `pdftocairo -svg` structural comparison and pass tolerances. Those vector
  comparisons are not yet run; the project does not claim full visual parity.
- [x] **Cover each repaired mismatch and hash its inputs.** The ten-fixture
  corpus already represents selectors, fixed content, transforms, typography,
  manual hyphens, tables, and bleed/crop boxes. Added Rust regressions cover
  auto-height fixed positioning and transformed inline boxes. The report hashes
  every HTML/CSS fixture and each referenced local resource.

### Completion criteria

- [x] All fixtures produce the same page count and page dimensions as the
  reference (10/10 fixtures; 17 pages on each side).
- [x] Extracted word and normalized line sequences match exactly for all
  fixtures; U+00AD/U+2010 handling is recorded in the report.
- [x] Centered line-box deltas are recorded for every text-identical line.
  Remaining fixture-specific values above 1 pt p95 or 2 pt max are documented
  with their affected fixtures and remain visible in the JSON.
- [x] Every visual category has a recorded measurement method and pass
  criteria. The SVG structural checks are specified but remain unmeasured; no
  unmeasured category is described as fully matching.
- [x] The pinned WeasyPrint baseline and repeat-run extracted-layout hashes
  reproduce on the documented environment.
