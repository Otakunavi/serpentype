# Serpentype 0.3.0

Serpentype 0.3.0 fixes layout defects found in the supplied compatibility fixture pack:

- Auto-width ancestor frames follow named-page dimensions, including landscape pages.
- Tables nested inside inline spans retain their columns and borders.
- Block HTML tags with `display: inline` or `inline-block` remain in the inline flow; empty inline-block placeholders and empty `<br>` line boxes are preserved.
- Unitless `line-height` inheritance scales with the computed font size, and collapsed table cells account for half-width borders in their layout and fragments.
- Page-margin `font` shorthand, universal CSS selectors, inline-only `column-count`, and identity CSS declarations are handled as described in the capability matrix.

## Fixture verification

The 19-fixture pack completed with four spawned workers: 19/19 Serpentype renders succeeded, with no failures, overflow, missing glyphs, unsupported-feature reports or diagnostics. The 2000-row table renders in 287 pages under both default limits and `max_pages=1000`.

For a fair comparison, the fixture's `common.css` was extended with a universal rule forcing its bundled `Fixture Sans` font for both engines. Against the bundled WeasyPrint 64.1 baseline, all 19 page counts now match. Page orientation and named landscape content width also match.

This is not pixel or pagination equivalence. Visual review and extracted text show different row-to-page assignments in the second tables of the appendix and notice-registration production templates, and some text line breaks/order differ. The fixtures therefore do not justify switching production rendering without reviewing those templates individually. WeasyPrint remains the production default.

The verification run used macOS arm64, CPython 3.13, Serpentype 0.3.0 and WeasyPrint 64.1. The common-font override is needed because the production snapshots contain inline font-family declarations that otherwise prevent a controlled comparison.

The local regression suites pass: `cargo test --locked --no-default-features` and `python -m pytest -q` (104 tests and 35 subtests).
