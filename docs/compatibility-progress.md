# Compatibility progress (unreleased)

The current development build fixes vertical line drift caused by nested inline elements with smaller font sizes. Inline runs retain their ancestor font metrics and line-height contribution, so a short nested run no longer shrinks the line and shifts following content. It also draws the winning collapsed-table border at a page-fragment boundary, so the last table row on a page keeps its bottom edge.

The supplied 0.2.1 fixture pack was rendered with WeasyPrint 64.1 and the same bundled Noto Sans font forced in both renderers. All 19 fixtures and 405 pages render; page counts and MediaBoxes match, with no Serpentype diagnostics, overflow, missing glyphs or unsupported-feature reports. The 2000-row table renders in 287 pages.

At 72 dpi, mean absolute grayscale pixel difference is 3.43 on a 0–255 scale, down from 3.78 for the published 0.5.0 build and 6.37 for 0.4.0. This is still visibly nonzero. Extracted word order differs on nine pages, even though normalized word contents match. **This work is not a release and must not be treated as exact WeasyPrint compatibility.** The version will not be published until these differences are resolved and verified.

Serpentype is AI-generated software, provided as-is and used at the user's own risk. Review and validate generated documents before relying on them.
