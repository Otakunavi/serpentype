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

Only the two import lines change in the `aliens` function. The same output types and target behavior are available: `write_pdf()` returns bytes with no target, writes and returns `None` for a binary file object or path, and `render().pages` supports `len()`. CSS and HTML accept `string`, `filename`, or `file_obj`. CSS `@font-face` rules using local TTF, CFF-outline OTF, WOFF and WOFF2 files (including `file://` URLs) are registered before layout. HTML local PNG/JPEG resources and base64 raster/SVG `data:` images work. Supported SVG is preserved as vector PDF content and uses registered fonts; Pillow is no longer needed.

## Rendering and compatibility boundary

`HTML` and `CSS` always render through Serpentype's Rust engine. There is no backend selector and no WeasyPrint import in the production package. The API preserves common call shapes and selected resource-loader conventions; it does not promise complete layout or PDF equivalence with WeasyPrint. Use the capability matrix and visual regression tests to check the supported subset.

WeasyPrint is an optional test dependency (`serpentype[test-weasy]`). The compatibility test runs Poppler `pdftotext -bbox-layout` on both PDFs, compares page count and line text, then compares line bounding boxes in points after centering the page boxes. It does not rasterize pages or use pixel-difference thresholds. Historical fixture-pack reports below describe an earlier release where production calls were delegated to WeasyPrint; they do not describe the current renderer.
- CSS `@font-face` sources in TTF, CFF-outline OTF, WOFF and WOFF2 are accepted. Remote fonts can be supplied through `resource_loader` or `url_fetcher`. The actual MinIO `fonts.css` and font files are not in this checkout, so their formats could not be verified.
- Remote HTML images can be supplied through `resource_loader` or `url_fetcher`; default loading remains local/data only. SVG external references are rejected, and font-dependent SVG text must use registered fonts.
- `Document.pages` supplies the collection length used by the application, but does not expose WeasyPrint's page box tree or drawing APIs. Imports from `weasyprint` itself remain WeasyPrint; switch the two imports to `serpentype` and `serpentype.text.fonts`.

The compatibility tests cover the `aliens` `BytesIO` path, `render().pages`, writing to a path, `file://` fonts, local/data PNG images, SVG QR placeholders, and a visual comparison with WeasyPrint. An in-memory run of the real `aliens/notice/html_to_pdf.py` with only its two imports changed returned a rewound PDF `BytesIO`; it used a local test stylesheet in place of the unavailable MinIO stylesheet.
