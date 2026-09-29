# Serpentype 0.1

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

For the cameral-control `page_definition.css`, pass the stylesheet after the application's Jinja variables and conditionals have been rendered. Register a font under `Times Newer Roman` before layout. The stylesheet's named pages, first-page margin boxes, page counters, QR flex column, and five-column grid are supported. Its `size: landscape` pages display horizontally; the redundant `page-orientation: rotate-right` does not rotate them again. Rotation still applies when the page size is not given by the `landscape` keyword.

## Install and build

Install from PyPI with `python -m pip install serpentype`. Prebuilt wheels are available for macOS arm64/x86_64, Linux x86_64, and Windows x86_64. On other platforms, pip will attempt a source build requiring Rust and maturin. You can also download a wheel from [GitHub Releases](https://github.com/Otakunavi/serpentype/releases). For development:

```sh
python3 -m venv .venv
.venv/bin/pip install maturin
.venv/bin/maturin develop --release
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --no-default-features
.venv/bin/python -I -m unittest discover -s tests -p 'test_api.py' -v
```

The CI workflow is configured to build wheels on Linux x86_64, macOS arm64/x86_64, and Windows x86_64. **Only macOS arm64 / CPython 3.13 wheel build and installation have been verified locally.** The wheel uses PyO3's CPython 3.10+ stable ABI. Serpentype's selected Rust dependencies do not link to browser engines, Pango, Cairo, or Fontconfig. WeasyPrint is needed only for the optional benchmark.

For the `cameral-control` WeasyPrint call sites, Serpentype also exports `HTML`, `CSS`, and `serpentype.text.fonts.FontConfiguration`. See the [API compatibility and migration limits](docs/weasyprint_compat.md).

See [capability matrix](docs/capabilities.md), [architecture](docs/architecture.md), [benchmark procedure and results](docs/benchmark.md), [page-definition stress test](output/benchmark/page_definition_stress/comparison.md), and [example invoice](examples/invoice.py). Fonts in `serpentype/assets` are Noto Sans under [SIL OFL 1.1](serpentype/assets/OFL.txt); the library code is MIT licensed.
