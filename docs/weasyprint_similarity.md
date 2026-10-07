# WeasyPrint similarity comparison

The comparison script renders ten checked-in HTML/CSS fixtures with Serpentype
and WeasyPrint, then uses Poppler to inspect page text, line boxes, embedded
fonts, and raster image objects. It does not rasterize or compare page pixels.

Run it from the repository root:

```sh
.venv/bin/python scripts/compare_weasyprint.py
```

The script writes [`output/compatibility/weasyprint-comparison.json`](../output/compatibility/weasyprint-comparison.json).
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

For colors, backgrounds, vector paths, glyph outlines, and border appearance,
the recorded non-pixel comparison method is to convert each PDF page to SVG
with Poppler `pdftocairo -svg`, normalize transforms and paint attributes, then
compare object classes, sRGB fill/stroke colors, stroke widths/dashes, image
placement, and flattened path geometry. Pass gates are exact object/color and
stroke-style agreement, image placement within 1 pt, and flattened path
distance at p95 <= 0.5 pt and max <= 1 pt. This method is specified for future
structural comparison; those categories are not measured by the current run
and no full visual parity claim is made.

## 0.8.0 canonical run

The run used Serpentype 0.8.0, WeasyPrint 64.1, Poppler `pdftotext` 26.05.0,
and Python 3.13.3 on macOS arm64. All ten fixtures have matching page counts,
page dimensions, word sequences, and normalized line sequences: 17 pages,
382 words, and 137 lines on each side. The equal-fixture mean line-box delta
is 0.64 pt.

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
