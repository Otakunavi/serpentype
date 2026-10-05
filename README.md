# Serpentype 0.2.0

This is the stable 0.2.0 release. The [capability matrix](docs/capabilities.md) identifies supported, partial and experimental features. The signed release tag, cross-platform artifacts, SBOM, provenance and checksums are available from the [GitHub release](https://github.com/Otakunavi/serpentype/releases/tag/v0.2.0); the [release checklist](docs/release-checklist-0.2.0.md) records verification details.

Serpentype is a Rust document layout engine with a Python API. It accepts a **controlled** HTML/CSS subset, calculates page breaks, then exports the prepared pages as a vector/text PDF. No browser, system renderer, or system font is used by Serpentype. A prebuilt platform wheel needs only Python ≥3.10.

```python
from serpentype import FontRegistry, Renderer, bundled_font_path

fonts = FontRegistry(cache_bytes=64 * 1024 * 1024)
fonts.register_file(bundled_font_path(), family="Noto Sans", weight=400)
fonts.register_file(bundled_font_path(700), family="Noto Sans", weight=700)
renderer = Renderer(fonts=fonts, base_dir=".")
prepared = renderer.layout(
    "<h1>Отчёт</h1><p>Привет, мир!</p>",
    css="@page { size: A4; margin: 18mm }",
)
print(prepared.page_count)
with open("report.pdf", "wb") as output:
    output.write(prepared.to_pdf())
```

The bundled Noto Sans TTF files are **not registered automatically**. Import them explicitly or provide your own font files/bytes. `Renderer.layout()` does not create a PDF. `PreparedDocument.to_pdf()` uses positioned items and resource bytes retained at layout time, so it can be repeated after source files are removed. Calls on one renderer may run concurrently from `ThreadPoolExecutor`; create a renderer inside each multiprocessing worker.

The development branch accepts registered TTF, CFF-outline OTF, WOFF and WOFF2 fonts. Variable `wght` and `wdth` fonts use the requested CSS weight and `font-stretch`; synthetic faces can be enabled with `Renderer(synthetic_bold=True, synthetic_italic=True)`. RustyBuzz shaping, grapheme fallback and Unicode bidi ordering are enabled by default; the former `experimental_shaping` keyword remains accepted for source compatibility. A missing glyph raises `MissingGlyphError` with structured code point, font request and source location fields. See the [font support boundary](docs/capabilities.md) before using these features in production.

Table pagination supports fixed/auto and constrained columns, `colspan`, fragmented `rowspan`, nested tables, and repeated `<thead>`/`<tfoot>` including connected spans. Oversized rowspan groups continue across pages between complete text lines. Use `tfoot { display: table-row-group }` when the footer should appear once. See the [capability matrix](docs/capabilities.md) for the exact border and nested-pagination boundaries.

The controlled CSS layout subset includes block-level relative/absolute/fixed positioning with repeated fixed content, integer z-order and translate/scale/rotate transforms. Flex supports reverse directions, wrapping, grow/shrink/basis and alignment; Grid supports fixed/percentage/`fr`/auto/minmax/repeat tracks, explicit placement and spans. Flex lines and Grid rows paginate atomically. Browser-complete intrinsic sizing, stacking contexts and named Grid lines are intentionally outside the supported boundary; see the [capability matrix](docs/capabilities.md).

The direct renderer and Python compatibility API accept a bounded resource loader for images, SVG and `@font-face` fonts:

```python
from serpentype import HTML, ResourceLoader

loader = ResourceLoader(
    mapping={"memory:logo": png_bytes},
    allowed_schemes=("memory", "data"),
    max_resource_bytes=2_000_000,
    max_total_bytes=10_000_000,
)
pdf = HTML(string="<img src='memory:logo'>", resource_loader=loader).write_pdf()
# The same loader can be passed to Renderer(resource_loader=loader).
```

`HTML` and `CSS` also accept `url_fetcher(url)` returning a WeasyPrint-style dictionary with `string`, `file_obj` or `filename` and optional `mime_type`. HTTP(S) is opt-in through `ResourceLoader(allowed_schemes=(..., "https"), allowed_hosts=(...))`. `Renderer(max_image_dpi=300)` can downsample oversized raster resources; supported SVG is preserved as vector PDF content.

For the cameral-control `page_definition.css`, pass the stylesheet after the application's Jinja variables and conditionals have been rendered. Register a font under `Times Newer Roman` before layout. The stylesheet's named pages, first-page margin boxes, page counters, QR flex column, and five-column grid are supported. Its `size: landscape` pages display horizontally; the redundant `page-orientation: rotate-right` does not rotate them again. Rotation still applies when the page size is not given by the `landscape` keyword.

## Install and build

Install from PyPI with `python -m pip install serpentype`. Prebuilt wheels are available for macOS arm64/x86_64, Linux x86_64, and Windows x86_64. On other platforms, pip will attempt a source build requiring Rust and maturin. You can also download a wheel from [GitHub Releases](https://github.com/Otakunavi/serpentype/releases). For development:

```sh
python3 -m venv .venv
.venv/bin/pip install maturin
.venv/bin/maturin develop --release
cargo fmt --all --check
cargo clippy --no-default-features --locked --all-targets -- -D warnings
cargo test --no-default-features --locked
.venv/bin/python -I -m unittest discover -s tests -p 'test_*.py' -v
```

The development CI workflow targets manylinux 2.17 and musllinux on x86_64/aarch64, macOS arm64/x86_64, Windows x86_64, and CPython 3.10–3.14 through ABI3. **Only macOS arm64 / CPython 3.13 wheel build and installation have been verified locally.** The declared Rust MSRV is 1.98.1. Serpentype's selected Rust dependencies do not link to browser engines, Pango, Cairo, or Fontconfig. WeasyPrint is needed only for the optional benchmark.

For the `cameral-control` WeasyPrint call sites, Serpentype also exports `HTML`, `CSS`, and `serpentype.text.fonts.FontConfiguration`. See the [API compatibility and migration limits](docs/weasyprint_compat.md).

See [capability matrix](docs/capabilities.md), [architecture](docs/architecture.md), [benchmark procedure and results](docs/benchmark.md), [page-definition stress test](output/benchmark/page_definition_stress/comparison.md), and [example invoice](examples/invoice.py). Fonts in `serpentype/assets` are Noto Sans under [SIL OFL 1.1](serpentype/assets/OFL.txt); the library code is MIT licensed.
