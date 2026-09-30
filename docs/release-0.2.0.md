# 0.2.0-alpha.1 prerelease status

Package metadata is 0.2.0-alpha.1. This prerelease exposes the current development work for evaluation while the final 0.2.0 release gates remain open. The atomic status and remaining count are maintained in the [0.2.0 release checklist](release-checklist-0.2.0.md).

## Delivered in the development branch

Styled inline fragments; Unicode grapheme-cluster font fallback, including tested monochrome emoji ZWJ sequences; CFF-outline OTF, WOFF/WOFF2, variable `wght`/`wdth` instances, CSS `font-stretch` and opt-in synthetic bold/italic; structured missing-glyph exception; opt-in RustyBuzz shaping for tested Latin, combining marks, Arabic, Hebrew, mixed LTR/RTL text and explicit bidi controls with logical PDF `/ActualText`; basic justify/indent/whitespace handling; contextual CSS lengths and opaque colors; percentage widths; `visibility`, paragraph `box-sizing` and paragraph/image min/max dimensions; fixed and auto table columns, `colspan`, body `rowspan`, cell vertical alignment, repeated headers/footers and paragraph `orphans`/`widows`; WebP/GIF decoding, raster soft masks, JPEG passthrough when orientation needs no correction, EXIF-oriented JPEG decoding and base64 raster `data:` images; `resvg` rendering for local and base64 SVG with configurable raster DPI; block `position: relative`, gap in the existing Flex/Grid subset, left/right/recto/verso page breaks and `@page :first/:left/:right/:blank`; bounded Python `ResourceLoader` and compatibility `url_fetcher`; external PDF links and internal destinations on block text and images; title/author/subject/keywords/language; diagnostics with occurrence counts; limits, cancellation and render stats. The legacy Python entry points remain callable.

## Verification completed locally

- Rust: 47 integration tests, no skipped tests, using `cargo test --no-default-features --locked`.
- Python: 62 tests pass against a wheel installed into a separate environment, including the compatibility adapter.
- `cargo fmt --all --check` and `cargo clippy --locked --all-targets -- -D warnings` pass locally.
- The published alpha commit passed the seven-target Linux, macOS and Windows wheel matrix and installed-wheel ABI3 tests on CPython 3.10–3.14. The post-alpha box-model commit has additionally passed a clean macOS arm64 wheel test; its full matrix remains a final-candidate gate.
- Repeated whole-document layouts produce byte-identical PDF output in the deterministic regression test.

## Performance comparison

The same 50-repeat corpus was run in separate processes against installed wheels from git `9569d5e` and this development branch on macOS arm64, CPython 3.13. Values are warm medians in milliseconds; the comparison is a local measurement, not a cross-platform guarantee.

| Document | Layout before → after | PDF export before → after | PDF bytes before → after |
| --- | ---: | ---: | ---: |
| Simple | 0.288 → 0.132 | 0.168 → 0.167 | 14,068 → 14,066 |
| Forced break | 0.964 → 0.366 | 0.285 → 0.285 | 14,403 → 14,393 |
| Table | 2.714 → 1.683 | 0.604 → 0.613 | 20,934 → 20,820 |

Peak RSS was 46.3 MB before and 48.3 MB after (+4.3%) in the earlier processes. The latest isolated comparison used 1,000 repeats, the same corpus and Python environment, and sequentially installed wheels before and after grapheme fallback and `font-stretch`. Warm layout/export medians in ms were: simple 0.135/0.190 → 0.137/0.192; forced break 0.348/0.333 → 0.350/0.334; table 1.770/0.703 → 1.789/0.709. These remain inside the 15% layout and 10% export gates locally. Peak RSS was 56.9 → 52.0 MB, though process RSS is noisy. After explicit bidi-control support, the opt-in shaping path measured 1.319, 4.233 and 10.060 ms warm layout on simple, forced-break and table documents, respectively, versus 1.414, 4.409 and 10.529 ms in the preceding run. Export medians were 0.193, 0.338 and 0.730 ms. The completed table rowspan/footer/vertical-align slice was measured separately over 100 repeats against the immediately preceding installed wheel: the table warm layout median changed from 1.696 to 1.929 ms (+13.7%), while export changed from 0.670 to 0.672 ms (+0.3%); simple and forced-break changes stayed near 1%. Peak RSS changed from 61.1 to 64.7 MB (+5.8%). The result stays inside the 15% layout, 10% export and 20% RSS gates. The benchmark includes separate cold and warm render, layout, export, thread throughput, RSS and PDF size; multi-process throughput remains to be added. The locally verified macOS arm64 ABI3 alpha wheel SHA256 is `4a415080317bfcee37af20ff2cd16c97765cebe516e0d1361fc723fa0e4c2bca` (normalized package version 0.2.0a1). Cross-platform release checksums are generated separately from the CI artifacts.

## Release blockers

The remaining blockers are tracked as individual unchecked items in the [release checklist](release-checklist-0.2.0.md). Completed items are retained there so each implementation block visibly reduces the open count. Partial capability labels are boundaries for callers and are not used as the progress counter.

The release should be tagged only after the capability matrix, mandatory CI fixtures, performance gates and distribution artifacts are complete. The tested Rust toolchain and declared MSRV are 1.98.1.
