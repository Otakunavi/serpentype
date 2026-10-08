"""Exercise Serpentype call shapes and test-only Poppler/WeasyPrint comparison."""

import base64
from io import BytesIO, StringIO
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

from serpentype import CSS, HTML
from serpentype.text.fonts import FontConfiguration
from serpentype import bundled_font_path


ROOT = Path(__file__).resolve().parents[1]


class WeasyCompatTests(unittest.TestCase):
    def test_backend_argument_is_not_part_of_the_production_api(self):
        with self.assertRaisesRegex(TypeError, "backend"):
            HTML(string="<p>Rust only</p>", backend="weasyprint")

    def test_compat_api_and_weasy_reference_match_poppler_text_geometry(self):
        try:
            from weasyprint import CSS as WeasyCSS, HTML as WeasyHTML
            from weasyprint.text.fonts import FontConfiguration as WeasyFontConfiguration
        except ImportError:
            self.skipTest("install the test-weasy extra to run the visual comparison")

        pdftotext = shutil.which("pdftotext")
        if pdftotext is None:
            pdftoppm = shutil.which("pdftoppm")
            candidates = []
            if pdftoppm:
                resolved_ppm = Path(pdftoppm).resolve()
                candidates.append(resolved_ppm.with_name("pdftotext"))
                if len(resolved_ppm.parents) > 2:
                    candidates.append(resolved_ppm.parents[2] / "native/poppler/bin/pdftotext")
                    candidates.append(resolved_ppm.parents[2] / "native/poppler/poppler/bin/pdftotext")
            sibling = next((path for path in candidates if path.is_file()), None)
            if sibling is None:
                self.skipTest("Poppler pdftotext is required for bbox comparison")
            pdftotext = str(sibling)

        fixture = ROOT / "tests" / "fixtures" / "visual_release_reference.html"
        stylesheet = ROOT / "tests" / "fixtures" / "visual_release_reference.css"
        font_face = (
            f"@font-face {{ font-family: 'Noto Sans'; src: url('{Path(bundled_font_path()).as_uri()}'); }}"
            f"@font-face {{ font-family: 'Noto Sans'; src: url('{Path(bundled_font_path(700)).as_uri()}'); font-weight: 700; }}"
        )
        css_text = font_face + "\n" + stylesheet.read_text()
        source = fixture.read_text()

        native_fonts = FontConfiguration()
        native_css = CSS(string=css_text, font_config=native_fonts)
        native_pdf = HTML(string=source).write_pdf(
            stylesheets=[native_css], font_config=native_fonts
        )

        weasy_fonts = WeasyFontConfiguration()
        weasy_css = WeasyCSS(
            string=css_text,
            base_url=fixture.parent.as_uri() + "/",
            font_config=weasy_fonts,
        )
        weasy_pdf = WeasyHTML(string=source, base_url=fixture.parent.as_uri() + "/").write_pdf(
            stylesheets=[weasy_css], font_config=weasy_fonts
        )
        def poppler_layout(pdf):
            result = subprocess.run(
                [pdftotext, "-bbox-layout", "-", "-"],
                input=pdf,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            root = ET.fromstring(result.stdout)
            ns = "{http://www.w3.org/1999/xhtml}"
            pages = []
            for page in root.findall(".//" + ns + "page"):
                lines = []
                for line in page.findall(".//" + ns + "line"):
                    words = line.findall(ns + "word")
                    text = " ".join("".join(word.itertext()) for word in words)
                    if text:
                        coords = tuple(float(line.get(key)) for key in
                                       ("xMin", "yMin", "xMax", "yMax"))
                        lines.append((text, coords))
                pages.append((float(page.get("width")), float(page.get("height")), lines))
            return pages

        native_pages = poppler_layout(native_pdf)
        weasy_pages = poppler_layout(weasy_pdf)
        self.assertEqual(len(native_pages), len(weasy_pages))
        self.assertEqual(len(native_pages), 1)
        native_width, native_height, native_lines = native_pages[0]
        weasy_width, weasy_height, weasy_lines = weasy_pages[0]
        self.assertEqual([line[0] for line in native_lines], [line[0] for line in weasy_lines])

        # Align the different MediaBoxes by page center, then compare Poppler's
        # extracted line boxes in points instead of comparing raster pixels.
        deltas = []
        for (_, native_box), (_, weasy_box) in zip(native_lines, weasy_lines):
            for index, (native_coord, weasy_coord) in enumerate(zip(native_box, weasy_box)):
                native_extent = native_width if index in (0, 2) else native_height
                weasy_extent = weasy_width if index in (0, 2) else weasy_height
                deltas.append(abs(
                    (native_coord - native_extent / 2)
                    - (weasy_coord - weasy_extent / 2)
                ))
        self.assertLessEqual(sum(deltas) / len(deltas), 5.0)
        self.assertLessEqual(max(deltas), 15.0)

    def test_inline_block_production_fixtures_render_with_both_engines(self):
        try:
            from weasyprint import CSS as WeasyCSS, HTML as WeasyHTML
            from weasyprint.text.fonts import FontConfiguration as WeasyFontConfiguration
        except ImportError:
            self.skipTest("install the test-weasy extra to render the reference PDFs")

        fixtures = ROOT / "tests" / "fixtures"
        base_url = fixtures.as_uri() + "/"
        regular = Path(bundled_font_path()).resolve()
        bold = Path(bundled_font_path(700)).resolve()
        font_css = (
            f"@font-face {{ font-family: 'Noto Sans'; src: url('{regular.as_uri()}'); }}\n"
            f"@font-face {{ font-family: 'Noto Sans'; src: url('{bold.as_uri()}'); font-weight: 700; }}\n"
        )
        for name in ("inline_block_width_notice", "inline_block_width_appendix"):
            with self.subTest(fixture=name):
                html = (fixtures / f"{name}.html").read_text(encoding="utf-8")
                css_text = font_css + (fixtures / f"{name}.css").read_text(encoding="utf-8")
                native_fonts = FontConfiguration()
                native_css = CSS(string=css_text, base_url=base_url, font_config=native_fonts)
                document = HTML(string=html, base_url=base_url).render(
                    stylesheets=[native_css], font_config=native_fonts
                )
                diagnostic_codes = {diagnostic.code for diagnostic in document.diagnostics}
                self.assertIn("inline-block-width", diagnostic_codes)
                width_warning = next(
                    diagnostic for diagnostic in document.diagnostics
                    if diagnostic.code == "inline-block-width"
                )
                self.assertEqual(width_warning.severity, "warning")
                self.assertGreater(len(document.pages), 0)
                native_pdf = document.write_pdf()
                self.assertTrue(native_pdf.startswith(b"%PDF-"))

                weasy_fonts = WeasyFontConfiguration()
                weasy_css = WeasyCSS(
                    string=css_text, base_url=base_url, font_config=weasy_fonts
                )
                reference = WeasyHTML(string=html, base_url=base_url).write_pdf(
                    stylesheets=[weasy_css], font_config=weasy_fonts
                )
                self.assertTrue(reference.startswith(b"%PDF-"))
                from pypdf import PdfReader

                self.assertEqual(
                    len(document.pages),
                    len(PdfReader(BytesIO(reference)).pages),
                )

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

    def test_table_presentational_hints_are_applied_only_when_requested(self):
        from pypdf import PdfReader

        source = HTML(string=(
            "<table width='160' height='80' cellpadding='6' cellspacing='4' border='2'>"
            "<tr><td align='right' valign='bottom'>A</td><td>B</td></tr></table>"
        ))
        without_hints = source.write_pdf(presentational_hints=False)
        with_hints = source.write_pdf(presentational_hints=True)
        self.assertNotEqual(without_hints, with_hints)
        plain_stream = PdfReader(BytesIO(without_hints)).pages[0].get_contents().get_data()
        hinted_stream = PdfReader(BytesIO(with_hints)).pages[0].get_contents().get_data()
        self.assertGreater(hinted_stream.count(b" re"), plain_stream.count(b" re"))

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
        pdf = HTML(
            string=f'<img src="data:image/svg+xml;base64,{encoded}">',
        ).write_pdf()
        self.assertIn(b"/Subtype /Form", pdf)
        path_svg = base64.b64encode(
            b'<svg width="20" height="20"><path d="M0 0L20 20"/></svg>'
        ).decode()
        path_pdf = HTML(
            string=f'<img src="data:image/svg+xml;base64,{path_svg}">',
        ).write_pdf()
        self.assertIn(b"/Subtype /Form", path_pdf)
        malformed = base64.b64encode(b'<svg><path').decode()
        with self.assertRaisesRegex(ValueError, "SVG"):
            HTML(
                string=f'<img src="data:image/svg+xml;base64,{malformed}">',
            ).write_pdf()


if __name__ == "__main__":
    unittest.main()
