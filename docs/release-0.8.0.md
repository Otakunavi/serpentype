# Serpentype 0.8.0

This release removes WeasyPrint delegation from the production `HTML`/`CSS`
API and improves the Rust renderer's fixed positioning, transformed inline
boxes, manual soft hyphens, and PDF bleed/crop page geometry.

## Compatibility comparison

The checked-in ten-fixture corpus is compared with WeasyPrint 64.1 using
Poppler `pdftotext -bbox-layout`. All fixtures match page counts, page
dimensions, extracted word sequences, and normalized line sequences. The
equal-fixture mean line-box delta is 0.64 pt. Documented geometry exceptions
remain in transformed layout, nested tables, and page selectors; the report
also specifies a structural SVG method for the visual features that Poppler
text extraction does not measure. Full visual parity is not claimed.

See [the detailed report](weasyprint_similarity.md), [the remaining TODO](../TODO.md),
and [API compatibility limits](weasyprint_compat.md).
