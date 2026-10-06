# Serpentype 0.3.0

Serpentype 0.3.0 fixes layout defects exposed by the supplied compatibility fixture pack:

- Auto-width ancestor frames now follow named-page dimensions, including landscape pages.
- Tables nested inside inline spans retain their table layout, cell borders and column structure.
- Page-margin `font` shorthand is supported. Identity CSS declarations no longer emit unsupported-feature diagnostics.
- Inline-only `column-count` content is laid out in balanced columns; unsupported block descendants are diagnosed.
- Universal CSS selectors are supported so a reset stylesheet can override inline normal declarations when explicitly marked `!important`.

## Compatibility findings

The 19-fixture pack completed in four spawned processes in both the default configuration and with `max_pages=1000`. The 2000-row document now renders in 287 pages and stays under the default 500-page limit; the pack's earlier default-limit failure is no longer expected after the landscape-width fix.

For a fair renderer comparison, the pack's `common.css` must force its bundled `Fixture Sans` font on all elements. Its original `html, body` rule does not override the fixture templates' inline font-family declarations in WeasyPrint. With the shared font forced, 11 of 19 fixture pairs have matching page counts. Eight production-template pairs still differ by one or two pages. Serpentype is not a drop-in WeasyPrint replacement; check page counts and render output for each production template before deployment.

The local compatibility run used WeasyPrint 70.0 and Serpentype 0.3.0 on macOS arm64 / CPython 3.13. The bundled baselines use WeasyPrint 64.1, so they are useful references rather than a cross-version release gate.
