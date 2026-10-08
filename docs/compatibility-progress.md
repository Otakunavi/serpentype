# Compatibility verification (0.7.0)

Historical record: the fixture run below exercised the original 0.7.0 delegated backend. The current production API no longer delegates to WeasyPrint; the test suite keeps it as an optional visual reference for compatibility work.

The supplied `serpentype-fixture-pack-0.2.1` was rendered with WeasyPrint 64.1 as the reference and with Serpentype's `HTML`/`CSS` compatibility API using `backend="weasyprint"`. In that historical release, the selected mode delegated to WeasyPrint while the standalone Rust renderer remained the default.

All 19 fixtures completed in four spawned workers. Both runs produced 38 successful render records and zero failures. Page counts match across all 353 pages, including the 2000-row table at 251 pages. For every fixture, the output PDF's SHA-256 and byte length match the direct WeasyPrint baseline exactly. This verifies byte-identical PDF output for the supplied templates in the tested environment.

The comparison used macOS arm64, CPython 3.13, WeasyPrint 64.1, and the supplied fixture pack unmodified. The optional `weasy-compat` extra pins WeasyPrint 64.1. The standalone Rust renderer remains a separate implementation; it is not pixel-identical to WeasyPrint for this fixture pack.

Serpentype is AI-generated software, provided as-is and used at the user's own risk. Review and validate generated documents before relying on them.
