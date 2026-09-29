"""End-to-end checks against the installed wheel and an optional independent PDF tool."""
import os
import re
import shutil
import subprocess
import tempfile
import unittest
import zlib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from serpentype import FontRegistry, Renderer, bundled_font_path

PDFINFO = shutil.which("pdfinfo")
PDFTOTEXT = shutil.which("pdftotext")


def registry():
    fonts = FontRegistry(cache_bytes=2 * 1024 * 1024)
    fonts.register_file(bundled_font_path(), family="Noto Sans")
    fonts.register_bytes(Path(bundled_font_path(700)).read_bytes(), family="Noto Sans", weight=700)
    return fonts


def pdf_pages(data):
    if not PDFINFO:
        return None
    with tempfile.NamedTemporaryFile(suffix=".pdf") as handle:
        handle.write(data)
        handle.flush()
        text = subprocess.check_output([PDFINFO, handle.name], text=True)
    return int(re.search(r"^Pages:\s*(\d+)", text, re.M).group(1))


def extracted(data, page=None):
    if not PDFTOTEXT:
        return None
    with tempfile.NamedTemporaryFile(suffix=".pdf") as handle:
        handle.write(data)
        handle.flush()
        args = [PDFTOTEXT]
        if page:
            args += ["-f", str(page), "-l", str(page)]
        return subprocess.check_output(args + [handle.name, "-"], text=True)


class ApiTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fonts = registry()
        cls.renderer = Renderer(fonts=cls.fonts)

    def test_cyrillic_and_repeat_export(self):
        doc = self.renderer.layout("<h1>Привет, мир!</h1><p>Тестовый документ.</p>")
        self.assertEqual(doc.page_count, 1)
        self.assertEqual(doc.to_pdf(), doc.to_pdf())
        self.assertEqual(pdf_pages(doc.to_pdf()), 1)
        if PDFTOTEXT:
            self.assertIn("Привет, мир!", extracted(doc.to_pdf()))

    def test_css_break_and_no_magic_class(self):
        html = '<p>Один</p><div class="pagebreaker"></div><p>Два</p>'
        self.assertEqual(self.renderer.layout(html).page_count, 1)
        doc = self.renderer.layout(html, ".pagebreaker { break-after: page }")
        self.assertEqual(doc.page_count, 2)
        self.assertEqual(pdf_pages(doc.to_pdf()), 2)
        self.assertEqual(self.renderer.layout('<p>Один</p><div style="break-before:page"><p>Два</p></div>').page_count, 2)

    def test_avoid_moves_fitting_paragraph(self):
        css = "@page { margin: 10pt; size: 150pt 100pt } p { margin: 0; font-size: 10pt; line-height: 20pt }"
        html = "<p>Первый<br>Первый<br>Первый</p><p style='break-inside:avoid'>Второй<br>Второй</p>"
        doc = self.renderer.layout(html, css)
        self.assertEqual(doc.page_count, 2)
        if PDFTOTEXT:
            self.assertNotIn("Второй", extracted(doc.to_pdf(), 1))
            self.assertIn("Второй", extracted(doc.to_pdf(), 2))

    def test_table_header_and_long_row(self):
        css = "@page { size: 150mm 90mm; margin: 8mm } th, td { border: 0.5pt solid black; padding: 2pt; font-size: 10pt } th { background: #dddddd }"
        rows = "".join(f"<tr><td>{i}</td><td>Строка {i} с текстом</td></tr>" for i in range(40))
        html = "<table><thead><tr><th>Номер</th><th>Описание</th></tr></thead><tbody>" + rows + "</tbody></table>"
        doc = self.renderer.layout(html, css)
        self.assertGreater(doc.page_count, 1)
        data = doc.to_pdf()
        self.assertEqual(pdf_pages(data), doc.page_count)
        if PDFTOTEXT:
            for page in range(1, doc.page_count + 1):
                self.assertIn("Описание", extracted(data, page))
        huge = "очень длинный текст " * 180
        doc = self.renderer.layout(f"<table><thead><tr><th>Шапка</th></tr></thead><tbody><tr><td>{huge}</td></tr></tbody></table>", css)
        self.assertGreater(doc.page_count, 2)
        self.assertEqual(pdf_pages(doc.to_pdf()), doc.page_count)
        if PDFTOTEXT:
            self.assertIn("длинный текст", extracted(doc.to_pdf(), doc.page_count))

    def test_boundary_empty_and_oversized_header(self):
        css = "@page { size: 100pt 100pt; margin: 10pt } p { margin: 0; font-size: 10pt; line-height: 20pt }"
        doc = self.renderer.layout("<p>А<br>Б<br>В<br>Г</p>", css)
        self.assertEqual(doc.page_count, 1)
        self.assertEqual(self.renderer.layout("").page_count, 1)
        self.assertEqual(self.renderer.layout("<table></table>").page_count, 1)
        with self.assertRaisesRegex(ValueError, "header"):
            self.renderer.layout("<table><thead><tr><th>" + "Слово " * 300 + "</th></tr></thead><tbody><tr><td>x</td></tr></tbody></table>", css)

    def test_font_cache_and_snapshot(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "font.ttf"
            path.write_bytes(Path(bundled_font_path()).read_bytes())
            fonts = FontRegistry(cache_bytes=1_000_000)
            fonts.register_file(str(path), family="Noto Sans")
            fonts.register_bytes(path.read_bytes(), family="Alias")
            self.assertGreaterEqual(fonts.cache_stats().hits, 1)
            doc = Renderer(fonts=fonts).layout('<p style="font-family: Missing, Noto Sans">Кириллица</p>')
            first = doc.to_pdf()
            path.unlink()
            fonts.clear_cache()
            self.assertEqual(doc.to_pdf(), first)
            self.assertEqual(Renderer(fonts=fonts).layout("<p>Повтор</p>").page_count, 1)
            path.write_bytes(Path(bundled_font_path(700)).read_bytes())
            fonts.register_file(str(path), family="Noto Sans", weight=400)
            self.assertGreaterEqual(fonts.cache_stats().misses, 2)
            changed = Renderer(fonts=fonts).layout("<p>Кириллица</p>").to_pdf()
            self.assertNotEqual(changed, first)

    def test_font_face_and_images(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / "local.ttf").write_bytes(Path(bundled_font_path()).read_bytes())
            for name in ("blue.png", "blue.jpg"):
                (root / name).write_bytes((Path(__file__).parent / "fixtures" / name).read_bytes())
            renderer = Renderer(fonts=FontRegistry(), base_dir=str(root))
            css = "@font-face { font-family: Local; src: url('local.ttf'); font-weight: 400; font-style: normal } p { font-family: Local }"
            doc = renderer.layout("<p>Кириллица</p><img src='blue.png'><img src='blue.jpg'>", css)
            for file in root.iterdir():
                file.unlink()
            pdf = doc.to_pdf()
            self.assertEqual(doc.to_pdf(), pdf)
            self.assertEqual(pdf.count(b"/Subtype /Image"), 2)
            self.assertEqual(pdf_pages(pdf), doc.page_count)

    def test_strict_diagnostics(self):
        doc = self.renderer.layout("<p style='float:left'>Text</p>")
        self.assertEqual(doc.warnings[0].code, "css-property")
        with self.assertRaisesRegex(ValueError, "float"):
            Renderer(fonts=self.fonts, strict=True).layout("<p style='float:left'>Text</p>")
        with self.assertRaisesRegex(ValueError, "JavaScript"):
            Renderer(fonts=self.fonts, strict=True).layout("<script>alert(1)</script><p>Text</p>")

    def test_threads(self):
        def task(n):
            doc = self.renderer.layout(f"<p>Поток {n}</p>")
            return doc.page_count, extracted(doc.to_pdf())
        with ThreadPoolExecutor(max_workers=8) as pool:
            result = list(pool.map(task, range(16)))
        self.assertTrue(all(count == 1 for count, _ in result))
        if PDFTOTEXT:
            for i, (_, text) in enumerate(result):
                self.assertIn(f"Поток {i}", text)

    def test_page_definition_css_when_source_is_available(self):
        path = Path("/Users/mini/Documents/dsrc/cameral-control/app/bg_worker/pdf_forming/templates/page_definition.css")
        if not path.exists():
            self.skipTest("source project stylesheet is unavailable")
        css = re.sub(
            r'{% if cameral.document_type == "ADVICE" %}(.*?){% else %}(.*?){% endif %}',
            lambda match: match.group(2),
            path.read_text(),
        )
        css = css.replace("{{ cameral.notice.reg_number }}", "123").replace(
            "{{ cameral.notice.reg_date }}", "28.09.2026"
        )
        fonts = registry()
        fonts.register_file(bundled_font_path(), family="Times Newer Roman")
        renderer = Renderer(fonts=fonts, strict=True)
        html = ('<article data-print="paged">'
                '<section class="appendix_1_ru"><p>Первая</p>'
                '<p style="break-before:page">Вторая</p></section>'
                '<section class="appendix_2_ru"><p>Третья</p></section>'
                '</article>')
        doc = renderer.layout(html, css)
        self.assertEqual(doc.page_count, 3)
        self.assertFalse(doc.warnings)
        pdf = doc.to_pdf()
        self.assertNotIn(b"/Rotate 90", pdf)
        if PDFINFO:
            with tempfile.NamedTemporaryFile(suffix=".pdf") as handle:
                handle.write(pdf)
                handle.flush()
                info = subprocess.check_output(
                    [PDFINFO, "-f", "1", "-l", "3", handle.name], text=True
                )
            self.assertRegex(info, r"Page\s+3 size:\s+841\.89 x 595\.28")

    def test_margin_boxes_have_separate_positions_and_subset_font(self):
        css = ('@page { size:A4; margin:15mm; '
               '@bottom-left { content:"Long left footer with registration reference"; font-size:8pt } '
               '@bottom-right { content:"Page " counter(page); font-size:8pt } }')
        pdf = self.renderer.layout("", css).to_pdf()
        streams = re.findall(rb"\nstream\n(.*?)\nendstream", pdf, re.S)
        content = [zlib.decompress(stream) for stream in streams if stream.startswith(b"x")]
        page_content = next(stream for stream in content if b"BT /F" in stream)
        positions = re.findall(
            rb"BT /F\d+ [\d.]+ Tf 1 0 0 1 ([\d.]+) ([\d.]+) Tm",
            page_content,
        )
        self.assertEqual(len(positions), 2)
        self.assertGreater(float(positions[1][0]) - float(positions[0][0]), 150)
        self.assertLess(len(pdf), 100_000)

    def test_all_six_margin_boxes_are_emitted_to_pdf(self):
        css = ('@page { size:300pt 300pt; margin:40pt; '
               '@top-left { content:"TL" } @top-center { content:"TC" } '
               '@top-right { content:"TR" } @bottom-left { content:"BL" } '
               '@bottom-center { content:"BC" } @bottom-right { content:"BR" } }')
        pdf = self.renderer.layout("", css).to_pdf()
        streams = re.findall(rb"\nstream\n(.*?)\nendstream", pdf, re.S)
        content = [zlib.decompress(stream) for stream in streams if stream.startswith(b"x")]
        page_content = next(stream for stream in content if b"BT /F" in stream)
        positions = [tuple(map(float, match)) for match in re.findall(
            rb"BT /F\d+ [\d.]+ Tf 1 0 0 1 ([\d.]+) ([\d.]+) Tm",
            page_content,
        )]
        self.assertEqual(len(positions), 6)
        top = sorted(x for x, y in positions if y > 260)
        bottom = sorted(x for x, y in positions if y < 40)
        self.assertEqual(len(top), 3)
        self.assertEqual(len(bottom), 3)
        for row in (top, bottom):
            self.assertLess(row[0], row[1])
            self.assertLess(row[1], row[2])


if __name__ == "__main__":
    unittest.main()
