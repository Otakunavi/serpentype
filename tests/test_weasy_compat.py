"""Exercise the WeasyPrint call forms used by cameral-control."""

import base64
from io import BytesIO, StringIO
from pathlib import Path
import tempfile
import unittest

from serpentype import CSS, HTML
from serpentype.text.fonts import FontConfiguration
from serpentype import bundled_font_path


ROOT = Path(__file__).resolve().parents[1]


class WeasyCompatTests(unittest.TestCase):
    def test_aliens_get_pdf_call_shape(self):
        fonts = FontConfiguration()
        font_url = Path(bundled_font_path()).as_uri()
        css = CSS(
            string=(
                '@font-face { font-family: "Notice"; '
                f'src: url("{font_url}") format("truetype"); font-weight: 400 }}'
                'body { font-family: "Notice" }'
            ),
            font_config=fonts,
            base_url=Path(bundled_font_path()).parent.as_uri() + "/",
        )
        output = BytesIO()
        result = HTML(file_obj=StringIO("<p>Проверка замены</p>")).write_pdf(
            output,
            stylesheets=[css],
            font_config=fonts,
            presentational_hints=True,
        )
        self.assertIsNone(result)
        self.assertTrue(output.getvalue().startswith(b"%PDF-"))
        self.assertLess(len(output.getvalue()), 50_000)

    def test_render_pages_and_write_to_path(self):
        fonts = FontConfiguration()
        stylesheet = CSS(string="@page { size: 120pt 100pt; margin: 10pt }", font_config=fonts)
        source = HTML(string="<p>one</p><p style='break-before:page'>two</p>")
        document = source.render(stylesheets=[stylesheet], font_config=fonts)
        self.assertEqual(len(document.pages), 2)
        pdf_bytes = document.write_pdf()
        self.assertTrue(pdf_bytes.startswith(b"%PDF-"))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "out.pdf"
            self.assertIsNone(document.write_pdf(path))
            self.assertEqual(path.read_bytes(), pdf_bytes)

    def test_file_base_url_and_data_png_image(self):
        image = (ROOT / "tests" / "fixtures" / "blue.png").read_bytes()
        encoded = base64.b64encode(image).decode("ascii")
        with tempfile.TemporaryDirectory() as directory:
            local_image = Path(directory) / "local.png"
            local_image.write_bytes(image)
            html = HTML(
                string=(
                    f'<img src="data:image/png;base64,{encoded}">'
                    '<img src="local.png">'
                ),
                base_url=directory,
            )
            pdf = html.write_pdf()
        self.assertTrue(pdf.startswith(b"%PDF-"))
        self.assertIn(b"/Subtype /Image", pdf)

    def test_svg_qr_placeholder_and_path(self):
        svg = (
            b'<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">'
            b'<g stroke="#000" stroke-width="4"><line x1="12" y1="12" x2="36" y2="12"/></g>'
            b'<text x="50%" text-anchor="middle"><tspan x="50%" y="60" font-size="22">QR</tspan></text>'
            b'</svg>'
        )
        encoded = base64.b64encode(svg).decode()
        pdf = HTML(string=f'<img src="data:image/svg+xml;base64,{encoded}">').write_pdf()
        self.assertIn(b"/Subtype /Image", pdf)
        path_svg = base64.b64encode(
            b'<svg width="20" height="20"><path d="M0 0L20 20"/></svg>'
        ).decode()
        path_pdf = HTML(string=f'<img src="data:image/svg+xml;base64,{path_svg}">').write_pdf()
        self.assertIn(b"/Subtype /Image", path_pdf)
        malformed = base64.b64encode(b'<svg><path').decode()
        with self.assertRaisesRegex(ValueError, "SVG"):
            HTML(string=f'<img src="data:image/svg+xml;base64,{malformed}">').write_pdf()


if __name__ == "__main__":
    unittest.main()
