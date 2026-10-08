# Serpentype 0.9.0

Serpentype 0.9.0 keeps more constrained layouts renderable and improves the
reference-comparison workflow. Inline-blocks without a usable content width
now produce a source-located warning and omit their contents instead of
failing PDF generation. Percentage heights resolve only against a definite
containing-block height. Height-constrained flex containers and flex rows that
cross page boundaries remain renderable, with warnings where row alignment is
omitted. A BOM/zero-width no-break space is excluded from visible text.

## Reference comparison

The pinned WeasyPrint 64.1 comparison now covers fourteen fixtures and 35 pages,
including the supplied notice and appendix templates and Russian and Kazakh
notices. All fixtures match page counts and page dimensions. The original ten
fixture corpus retains exact normalized word and line sequences. The production
templates still differ in extracted text and line wrapping; the QR inline-block
content is omitted when it has no usable width. Across the 35 pages, 14 meet the
documented raster tolerance. The remaining differences and per-line details
are in the [comparison report](weasyprint_similarity.md) and the
[machine-readable results](../output/compatibility/weasyprint-comparison.json).

The HTML [visual-difference report](../output/compatibility/visual-diff-report.html)
contains cropped examples. Raster scores are diagnostic signals and do not
establish full visual parity.

See [TODO.md](../TODO.md) for the remaining compatibility work and
[API compatibility limits](weasyprint_compat.md) for the supported boundary.
