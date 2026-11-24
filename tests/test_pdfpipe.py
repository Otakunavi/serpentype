"""Compare draft and signed notice PDFs against WeasyPrint 64.1."""

import base64
import importlib.util
import subprocess
import sys
from contextlib import closing
from pathlib import Path
import tempfile
import unittest


PIPELINE = Path(__file__).resolve().parent / "pdfpipe" / "render.py"
DEPENDENCIES = ("jinja2", "pypdf", "pypdfium2", "weasyprint", "zxingcpp")
DELIVERED = {"ru": "Уведомление вручено", "kk": "табысталды"}
DISPATCHED = {"ru": "Уведомление отправлено", "kk": "жіберілді"}
FOOTER = {
    "ru": "Согласно пункту 2 статьи 62",
    "kk": "Цифрлық кодексінің 62-бабының 2-тармағына сәйкес",
}
COUNTER = {"ru": "Лист {page} из 6", "kk": "{page} беттің 6"}
TEST_LINK = (
    "https://test-isna.kgd.gov.kz/filesaccess/files/audit.cameral-document-files/"
    "00000000-0000-4000-8000-00000000000{serial}-2025-mio"
)


def raster_pages(path):
    import pypdfium2

    with pypdfium2.PdfDocument(path) as document:
        for index in range(len(document)):
            with closing(document[index]) as page, closing(page.render(scale=3)) as bitmap:
                yield bitmap.to_pil().convert("L")


def dark_rows(image, box):
    """Return the start of each group of ink rows in the left footer."""
    crop = image.crop(box)
    rows = [crop.crop((0, y, crop.width, y + 1)).getextrema()[0] < 155
            for y in range(crop.height)]
    return [y for y, dark in enumerate(rows) if dark and (y == 0 or not rows[y - 1])]


class ImageCompatibilityTests(unittest.TestCase):
    def test_svg_mime_variants_match_canonical_svg(self):
        """Serpentype paints the same SVG for each declaration used by the notice pipeline."""
        from serpentype import HTML

        svg = base64.b64encode(
            b"<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>"
            b"<rect width='20' height='20' fill='black'/></svg>"
        ).decode("ascii")

        def render(mime):
            image = f"<img style='display:block;width:20pt' src='data:{mime};base64,{svg}'>"
            html = f"<div>{image}</div><div><span style='display:inline-block'>{image}</span></div>"
            return HTML(string=html).write_pdf()

        canonical = render("image/svg+xml")
        self.assertIn(b"/Subtype /Form", canonical)
        for mime in ("image/png", "image/svg"):
            with self.subTest(mime=mime):
                self.assertEqual(render(mime), canonical)

    def test_inline_image_diagnostic_and_limits(self):
        """Malformed inline data is recoverable and resource limits still fail rendering."""
        from serpentype import HTML, RenderLimits

        invalid = "<div><span style='display:inline-block'>"
        invalid += "<img style='width:20pt' src='data:image/png;base64,bm90IGEgcG5n'>"
        invalid += "</span>Content after the image</div>"
        document = HTML(string=invalid).render()
        self.assertEqual(len(document.pages), 1)
        self.assertIn("image-decode", [diagnostic.code for diagnostic in document.diagnostics])

        svg = base64.b64encode(b"<svg xmlns='http://www.w3.org/2000/svg' width='200' height='200'/>").decode()
        for limits, reason in (
            (RenderLimits(max_resource_bytes=4), "max_resource_bytes"),
            (RenderLimits(max_image_pixels=1), "max_image_pixels"),
        ):
            with self.subTest(limit=reason), self.assertRaisesRegex(ValueError, reason):
                HTML(string=f"<span><img src='data:image/svg+xml;base64,{svg}'></span>").render(limits=limits)


@unittest.skipUnless(all(importlib.util.find_spec(module) for module in DEPENDENCIES),
                     "install the test-pdfpipe extra to run both PDF engines and decode QR codes")
class PdfPipelineTests(unittest.TestCase):
    def test_notice_matrix(self):
        """Both languages preserve the watermark, delivery, footers, and draft/signed QR content."""
        from PIL import ImageChops
        from pypdf import PdfReader
        import zxingcpp

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            command = [sys.executable, str(PIPELINE), "--renderer", "both", "--mode", "both"]
            render = subprocess.run(
                [*command, "--output-dir", directory],
                cwd=directory,
                check=False, capture_output=True, text=True, timeout=120,
            )
            self.assertEqual(render.returncode, 0, render.stdout + render.stderr)

            for language in ("ru", "kk"):
                regions = {}
                footers = {}
                for engine in ("weasyprint", "serpentype"):
                    with self.subTest(language=language, engine=engine):
                        paths = {mode: root / mode / engine / f"{language}.pdf"
                                 for mode in ("draft", "signed")}
                        for path in paths.values():
                            with path.open("rb") as file:
                                pdf = PdfReader(file)
                                self.assertEqual(len(pdf.pages), 6)
                                for number, page in enumerate(pdf.pages, start=1):
                                    dimensions = (595.28, 841.89) if number < 6 else (841.89, 595.28)
                                    for actual, expected in zip((page.mediabox.width, page.mediabox.height),
                                                                dimensions):
                                        self.assertAlmostEqual(float(actual), expected, delta=0.03)
                                    text = " ".join(page.extract_text().split())
                                    self.assertIn(FOOTER[language], text)
                                    self.assertEqual(COUNTER[language].format(page=number) in text, number >= 5)
                                    if number == 4:
                                        # KK repeats a short substring elsewhere, so assert the
                                        # completed labels and their sequence on the delivery page.
                                        self.assertIn(DELIVERED[language], text)
                                        self.assertIn(DISPATCHED[language], text)
                                        self.assertLess(text.index(DELIVERED[language]), text.index(DISPATCHED[language]))
                                    elif number == 3:
                                        self.assertNotIn(DELIVERED[language], text)

                        for number, (draft, signed) in enumerate(
                            zip(raster_pages(paths["draft"]), raster_pages(paths["signed"])), start=1
                        ):
                            width, height = draft.size
                            for page in (draft, signed):
                                # Real texture reaches past the content margin and repeats in both axes.
                                for x, y in ((0.035, 0.04), (0.96, 0.90)):
                                    left, top = int(width * x), int(height * y)
                                    texture = page.crop((left, top, left + 30, top + 60)).histogram()
                                    self.assertGreater(sum(texture[242:251]), 150)
                                # The emblem is darker than the texture and crosses an
                                # otherwise empty band on every page of the notice.
                                emblem = page.crop((int(width * .40), int(height * .46),
                                                     int(width * .60), int(height * .60))).histogram()
                                self.assertGreater(sum(emblem[190:241]), 2500)
                            if number == 6:
                                footers[engine] = dark_rows(draft, (int(width * .07), height - 135,
                                                                   int(width * .82), height - 36))

                            difference = ImageChops.difference(draft, signed)
                            region = difference.point(lambda value: 255 if value > 100 else 0).getbbox()
                            if number not in (1, 4):
                                self.assertIsNone(region)
                                continue
                            self.assertIsNotNone(region)
                            regions[engine, number] = region
                            left, top, right, bottom = region
                            self.assertLess(right, width * .24)
                            self.assertAlmostEqual(right - left, 170 if number == 1 else 255, delta=3)
                            self.assertAlmostEqual(right - left, bottom - top, delta=3)
                            # Corner brackets distinguish a draft placeholder from an
                            # empty image or a label elsewhere in the search region.
                            placeholder = draft.crop(region)
                            w, h = placeholder.size
                            for x, y in ((0, 0), (.76, 0), (0, .76), (.76, .76)):
                                corner = placeholder.crop((int(w*x), int(h*y),
                                                           int(w*(x+.24)), int(h*(y+.24))))
                                self.assertGreater(sum(corner.histogram()[:130]), 60)
                            serial = (1 if language == "ru" else 2) if number == 1 else 3
                            found = zxingcpp.read_barcodes(signed, formats=zxingcpp.BarcodeFormat.QRCode)
                            self.assertEqual([code.text for code in found], [TEST_LINK.format(serial=serial)])

                if len(regions) != 4 or len(footers) != 2:
                    continue
                with self.subTest(language=language, comparison="positions"):
                    self.assertEqual(len(footers["serpentype"]), 2)
                    self.assertEqual(len(footers["weasyprint"]), 2)
                    for actual, expected in zip(footers["serpentype"], footers["weasyprint"]):
                        self.assertAlmostEqual(actual / 3, expected / 3, delta=2)
                    self.assertAlmostEqual(regions["serpentype", 1][1] / 3,
                                           regions["weasyprint", 1][1] / 3, delta=3)
                    # WeasyPrint 64.1 does not apply flex gap. At this width
                    # it wraps one fewer 14 pt line in the delivery row.
                    self.assertAlmostEqual(regions["serpentype", 4][1] / 3,
                                           regions["weasyprint", 4][1] / 3, delta=20)


if __name__ == "__main__":
    unittest.main()
