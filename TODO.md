# TODO

## 0.11.0 — Notice PDF pipeline compatibility

Baseline from 2026-11-23. The [local PDF pipeline](tests/pdfpipe/README.md)
reproduces dev notice template 7 (`4ea5b32d`) with synthetic RU and KK data.
The run used Serpentype 0.10.0, WeasyPrint 64.1, Python 3.12 on macOS arm64,
and the bundled Noto Sans font profile. Both engines received the same
assembled HTML and font CSS in each case. From `tests/pdfpipe`, run
`python render.py --renderer both --mode both --output-dir /tmp/pdfpipe-011`.

| Check | WeasyPrint 64.1 | Serpentype 0.10.0 |
| --- | --- | --- |
| Pages and orientation | Six pages in all four RU/KK × draft/signed cases. Appendix 1 is portrait on page 5, and appendix 2 is landscape on page 6. | Same page counts, dimensions, orientation, and appendix order. |
| Watermark (both modes, both languages) | Centered SVG emblem and repeating JPEG texture paint behind content on all six pages, including the landscape appendix. | Both layers are absent on all pages. |
| Draft QR placeholders | Both SVG placeholders paint on pages 1 and 4. | Both placeholders are missing. The top URI declares `image/png` for SVG bytes, and the lower URI uses `image/svg` instead of `image/svg+xml`. Changing each MIME type independently restores the corresponding placeholder. |
| Signed QR codes | The 20 mm PDF QR paints on page 1, and the 30 mm XML QR paints on page 4. | Same page numbers and dimensions. The rasterized 45 × 45 module grids match the expected PDF/XML links in both engines. The XML QR sits about 169 pt higher in RU and 238 pt higher in KK than in WeasyPrint. |
| Body flow | The delivery section starts on page 4 in RU and KK. | The delivery section starts on page 3 and continues onto page 4. The line wraps differ despite matching page counts. |
| Footers and counters | The legal footer appears on all pages. Localized counters on pages 5 and 6 are correct. On the landscape RU page, the legal footer uses two lines. | The footer and counters are present with the same text and values. The landscape RU footer wraps onto three lines, and its last line sits lower. |

### 0.11.0 verification

The compatibility suite generated and inspected all eight PDFs with the
original dev MIME declarations. It checks A4 page geometry, both
watermark layers, the two draft placeholders, delivery text order, footer
counters, and QR URLs read by ZXing from page rasters at 216 dpi. The bundled
Noto Sans profile was used. The dev font profile and the optional lower QR
for signature data remain outside this fixture.

- [x] **Recover from an invalid inline image.** The `HTML.render()` regression
  test verifies an image diagnostic and a valid PDF. Resource and pixel
  limits remain fatal, and rendering still checks cancellation.
- [x] **Paint layered `@page` backgrounds.** Both SVG and JPEG layers paint
  behind the content on all six pages, including `:first` and the named
  landscape appendix. Each layer has an independent raster assertion.
- [x] **Accept the captured SVG MIME variants in Serpentype.** Legacy
  `image/svg` and SVG bytes labeled `image/png` produce the same PDF as
  `image/svg+xml` in the library-level regression. The notice fixture passes
  its dev input directly to the renderer. Both engines decode the signed
  QR payloads from genuine PNG images.
- [x] **Keep the delivery section together on page 4.** Both languages retain
  the order of the delivery and dispatch columns. The lower QR is roughly
  18 pt below WeasyPrint 64.1 in both languages, within the 20 pt regression
  tolerance. WeasyPrint 64.1 ignores flex `gap` and wraps one fewer line in
  this layout.
- [x] **Match appendix margin boxes.** Named pages honor footer widths,
  line height, and bottom margin. The landscape footer uses two lines in
  both languages, with a 2 pt baseline tolerance. Localized counters and
  the first-page legal footer are covered.
- [x] **Update the PDF pipeline integration test.** The test uses the CLI's
  current output paths and decodes both signed QR URLs from each PDF.

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
- [ ] **Improve text-flow parity in the original production fixtures.** In the
  comparison with the shared Noto Sans font, word sequence similarity is 96.51%
  for the notice and 99.30% for the appendix. Normalized line sequence
  similarity is 57.91% and 79.88%. Page counts match. The JSON report records
  the line wrapping and extracted text differences, including omitted
  inline-block content. See the 0.11.0 section for the current dev pipeline.

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
