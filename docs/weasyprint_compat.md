# WeasyPrint call compatibility for cameral-control

`app/aliens/notice/html_to_pdf.py` is the central PDF entry point for the `aliens` package. It reads a rendered HTML `StringIO`, loads `fonts.css` from MinIO into a local font cache, builds one `CSS` object, and calls `HTML(file_obj=...).write_pdf(BytesIO, stylesheets=[...], font_config=..., presentational_hints=True)`. It returns a rewound `BytesIO` to the upload pipeline. `app/bg_worker/repository.py` also uses `HTML.render(...).pages`, `Document.write_pdf(BytesIO)`, `Document.write_pdf(path)`, and two CSS objects in one render. `app/generate_pdf/service.py` uses `HTML(file_obj=..., base_url=...).write_pdf(BytesIO, presentational_hints=True)`.

The `aliens` input comes from a Redis template, which Jinja renders in `template_to_html.py`. `pipeline.py` then passes the PDF stream to the MinIO upload method; that method uses `getbuffer().nbytes`, so preserving the `BytesIO` return type matters. The table builder in `pipeline.py` generates inline `word-break: break-all`, `overflow-wrap: anywhere`, `table-layout: fixed`, `border-collapse: collapse`, and `cellpadding`/`cellspacing` attributes. These are relevant to visual parity and are not all implemented by Serpentype.

Serpentype provides those call forms under `serpentype` and `serpentype.text.fonts`:

```python
from serpentype import CSS, HTML
from serpentype.text.fonts import FontConfiguration

font_config = FontConfiguration()
css = CSS(string=css_text, font_config=font_config, base_url=font_cache_url)
pdf_file = BytesIO()
HTML(file_obj=html_file).write_pdf(
    pdf_file,
    stylesheets=[css],
    font_config=font_config,
    presentational_hints=True,
)
```

Only the two import lines change in the `aliens` function. The same output types and target behavior are available: `write_pdf()` returns bytes with no target, writes and returns `None` for a binary file object or path, and `render().pages` supports `len()`. CSS and HTML accept `string`, `filename`, or `file_obj`. CSS `@font-face` rules using local TTF, CFF-outline OTF, WOFF and WOFF2 files (including `file://` URLs) are registered before layout. HTML local PNG/JPEG resources and base64 raster/SVG `data:` images work. SVG uses the core `resvg` raster fallback and registered fonts; Pillow is no longer needed.

## Limits before a production switch

- This is API compatibility for the call sites above, not full WeasyPrint rendering equivalence. `presentational_hints=True` is accepted, but HTML presentational attributes beyond Serpentype's controlled subset are not emulated. Complex HTML/CSS in templates from Redis needs a fixture-based visual comparison.
- The generated violation tables use `word-break`, `overflow-wrap`, `table-layout`, `border-collapse`, and HTML table attributes. Serpentype's table pagination works, but these layout details may produce different widths, wrapping, and borders. Compare real notices before switching the application imports.
- CSS `@font-face` sources in TTF, CFF-outline OTF, WOFF and WOFF2 are accepted. Remote fonts can be supplied through `resource_loader` or `url_fetcher`. The actual MinIO `fonts.css` and font files are not in this checkout, so their formats could not be verified.
- Remote HTML images can be supplied through `resource_loader` or `url_fetcher`; default loading remains local/data only. SVG is rasterized by `resvg`, with no vector PDF preservation; external references inside SVG are rejected. Font-dependent SVG text must use registered fonts.
- `Document.pages` supplies the collection length used by the application, but does not expose WeasyPrint's page box tree or drawing APIs. Imports from `weasyprint` itself remain WeasyPrint; switch the two imports to `serpentype` and `serpentype.text.fonts`.

The compatibility tests cover the `aliens` `BytesIO` path, `render().pages`, writing to a path, `file://` fonts, local/data PNG images, and the known SVG QR placeholders. An in-memory run of the real `aliens/notice/html_to_pdf.py` with only its two imports changed returned a rewound PDF `BytesIO`; it used a local test stylesheet in place of the unavailable MinIO stylesheet.
