# Serpentype 0.4.0

Serpentype 0.4.0 addresses the remaining layout defects found while exercising the supplied 19-fixture compatibility pack:

- Collapsible trailing spaces no longer cause a line to wrap early.
- A block that fragments across pages leaves the next sibling at the correct position on the final page.
- Inline wrappers that end with block-level content preserve the trailing line break.
- Auto-sized inline Flex items use their intrinsic text width; collapsed-table-cell text no longer receives an artificial width allowance.
- HTML table data cells use the default middle vertical alignment, while text inherits only properties that CSS defines as inherited.

## Compatibility verification

The 19 fixtures were rendered with four spawned workers using the supplied bundled font forced in both Serpentype and WeasyPrint 64.1. All Serpentype renders completed without failures, diagnostics, overflow, missing glyphs or unsupported-feature reports. Page counts and page boxes match in all cases, for a total of 405 pages; normalized text tokens match on every page. The 2000-row table renders in 287 pages in both engines.

PDF text extraction order still differs on nine pages. A full pixel-by-pixel visual comparison across the fixture pack has not been established, so WeasyPrint remains the production default. These results establish the tested compatibility subset; they do not claim complete browser or WeasyPrint equivalence.

The local regression suites pass: `cargo test --locked --no-default-features`, `cargo clippy --no-default-features --locked --all-targets -- -D warnings`, and `python -m pytest -q` (104 tests and 35 subtests). The fixture run used the fixture pack's forced-font copy and `RenderLimits(max_pages=1000)` for the long-table case.

Serpentype is AI-generated software, provided as-is and used at the user's own risk. Review and validate generated documents before relying on them.
