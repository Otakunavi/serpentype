# Reference renderer alignment

The comparison script renders the checked-in HTML fixtures with Serpentype and
WeasyPrint as the reference renderer, then uses Poppler to inspect page text,
line boxes, embedded fonts, raster image objects, and rasterized page
appearance. It supports companion CSS files and HTML with embedded styles. If
a renderer fails on one fixture, the report records its error and continues
with the other fixtures; aggregate metrics include only fixtures rendered by
both.

Run it from the repository root:

```sh
.venv/bin/python scripts/compare_weasyprint.py
```

The script writes [`output/compatibility/weasyprint-comparison.json`](../output/compatibility/weasyprint-comparison.json).
It also writes the [cropped visual difference report](../output/compatibility/visual-diff-report.html)
and its PNG crops to `output/compatibility/visual-diff-assets/`.
The report records fixture and local-resource SHA256 values, PDF and extracted
layout hashes, page dimensions, every extracted line and bbox, and Poppler's
font and image inventories. Install `.[test-weasy]` to use the pinned
WeasyPrint 64.1 reference.

## Comparison rules

- Page count and paired page dimensions are compared exactly.
- Word and line sequences use `difflib.SequenceMatcher` with `autojunk=False`.
  U+00AD is treated as invisible; a selected manual break compares by its
  painted U+2010 glyph. Raw extracted text remains in the report.
- Identical lines on the same page are compared by their four bbox coordinates
  after centering each coordinate system on its page. Values are absolute PDF
  points. p95 uses the sorted sample at index `floor(0.95 * (n - 1))`.
- `pdffonts` records font objects; `pdfimages -list` records image dimensions,
  color components, masks, and encoding. These object inventories do not prove
  equivalent glyph outlines, image appearance, or painting order.
- `pdftoppm` rasterizes each PDF at 96 DPI. The pages are centered on a white
  canvas sized to the larger image without scaling, then compared with Pillow.
  The report records the mean 8-bit grayscale difference, changed pixel
  fractions above 8 and 32, exact pixel fraction, and page dimensions. A page
  passes the visual tolerance when its mean difference is at most 3.01 and no
  more than 5% of pixels differ by over 8. These are comparison indicators;
  failing pages remain in the report and do not fail the script.
- The HTML report groups changed pixels into local crop regions. Each crop image
  shows the Serpentype excerpt, the WeasyPrint excerpt, and a difference
  heatmap. Captions include crop coordinates, changed-pixel count, and nearby
  extracted text from both PDFs. Small regions below 18 changed pixels are
  omitted; up to 30 regions are included per page.

Raster metrics help locate large visual differences, while the text and bbox
metrics help explain some of them. A raster score alone cannot identify
whether a difference comes from font rasterization, layout, colors, or vector
drawing.

## 0.8.0 canonical run

The original ten-fixture run used Serpentype 0.8.0, WeasyPrint 64.1, Poppler
`pdftotext` 26.05.0, and Python 3.13.3 on macOS arm64. All ten fixtures have
matching page counts, page dimensions, word sequences, and normalized line
sequences: 17 pages, 382 words, and 137 lines on each side. The equal-fixture
mean line-box delta is 0.64 pt.

| Fixture | Pages S/W | Lines S/W | Mean / p95 / max bbox delta |
| --- | ---: | ---: | ---: |
| Release reference | 1 / 1 | 7 / 7 | 0.28 / 0.89 / 1.08 pt |
| Layout | 1 / 1 | 21 / 21 | 1.69 / 5.26 / 10.24 pt |
| Resources | 2 / 2 | 3 / 3 | 0.15 / 0.47 / 0.90 pt |
| Tables | 2 / 2 | 36 / 36 | 0.18 / 0.30 / 3.38 pt |
| Table features | 1 / 1 | 16 / 16 | 2.42 / 11.99 / 11.99 pt |
| Page selectors | 4 / 4 | 24 / 24 | 0.73 / 2.69 / 3.77 pt |
| Typography | 2 / 2 | 10 / 10 | 0.06 / 0.15 / 0.60 pt |
| Fonts | 1 / 1 | 5 / 5 | 0.29 / 2.14 / 2.43 pt |
| Webfonts | 1 / 1 | 3 / 3 | 0.25 / 0.40 / 1.34 pt |
| Pagination | 2 / 2 | 12 / 12 | 0.31 / 1.69 / 1.69 pt |

The main layout exceptions are the transformed/flex/grid blocks in the layout
fixture (max 10.24 pt), auto-sized nested table columns in table features (max
11.99 pt), and margin-box placement in page selectors (max 3.77 pt). Each
exception remains visible per page and per line in the JSON. The renderer's
text and page geometry match this corpus, but the remaining bbox deltas and
unmeasured vector/paint categories mean full visual parity is not established.

## Production template fixtures

`inline_block_width_notice` and `inline_block_width_appendix` preserve the two
HTML files supplied for the inline-block width failure. The comparison forces
the same bundled Noto Sans face in both engines because the source templates
contain inline font-family declarations. Both engines now render both files to
four pages. Word sequence similarity is 96.51% for the notice and 99.30% for
the appendix; normalized line sequence similarity is 57.91% and 79.88%.

Serpentype emits `inline-block-width` warnings at the QR placeholder and omits
that inline-block's contents when it has no usable content width. These warnings
are visible in the JSON report alongside the page and text comparison results.

The Russian and Kazakh notice templates are also included as `notice_ru` and
`notice_kk`. In the current run, each engine produced five pages for each
template. Across all 35 compared pages, 14 meet the visual tolerance above;
the report retains per-page metrics for the other 21 pages as well.

## 0.9.0 comparison run

The run used the local 0.9.0 development build, WeasyPrint 64.1, Poppler
`pdftotext` 26.05.0, and Python 3.13.3 on macOS arm64. All fourteen fixtures
rendered successfully and match page counts and dimensions: 35 pages on each
side. Ten fixtures retain exact normalized line sequences. Across the corpus,
word-sequence similarity is 99.67% and line-sequence similarity is 93.29%.
Fourteen of 35 pages meet the raster tolerance; the other pages and their
cropped difference regions remain recorded in the JSON and HTML reports.

The production-template QR content and text-flow differences remain open, as
tracked in [TODO.md](../TODO.md). These measurements are a fixture-specific
comparison, not a claim of complete visual compatibility.
