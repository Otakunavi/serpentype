"""End-to-end test for the local production-shaped notice rendering pipeline."""

import importlib.util
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PIPELINE = ROOT / "tests" / "pdfpipe" / "render.py"
PDFINFO = shutil.which("pdfinfo")
PDFTOPPM = shutil.which("pdftoppm")


class PdfPipelineTests(unittest.TestCase):
    def test_russian_and_kazakh_notices_render_to_valid_pdfs(self):
        if importlib.util.find_spec("qrcode") is None:
            self.skipTest("install the test-pdfpipe extra to render the notice fixtures")

        with tempfile.TemporaryDirectory() as output_dir:
            result = subprocess.run(
                [sys.executable, str(PIPELINE), "both", "--output-dir", output_dir],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
                timeout=300,
            )

            for language in ("ru", "kk"):
                with self.subTest(language=language):
                    pdf = Path(output_dir) / f"{language}.pdf"
                    self.assertTrue(pdf.is_file(), result.stdout)
                    data = pdf.read_bytes()
                    self.assertTrue(data.startswith(b"%PDF-"))
                    self.assertGreater(len(data), 10_000)

                    if PDFTOPPM:
                        raster_prefix = Path(output_dir) / f"{language}-first-page"
                        subprocess.run(
                            [
                                PDFTOPPM,
                                "-f",
                                "1",
                                "-l",
                                "1",
                                "-singlefile",
                                "-png",
                                "-scale-to",
                                "1200",
                                str(pdf),
                                str(raster_prefix),
                            ],
                            check=True,
                            capture_output=True,
                            timeout=60,
                        )
                        from PIL import Image

                        with Image.open(f"{raster_prefix}.png") as page:
                            gray = page.convert("L")
                            width, height = gray.size
                            qr_area = gray.crop(
                                (int(width * 0.04), int(height * 0.03),
                                 int(width * 0.28), int(height * 0.25))
                            )
                            dark_pixels = sum(pixel < 64 for pixel in qr_area.getdata())
                            dark_ratio = dark_pixels / (qr_area.width * qr_area.height)
                            self.assertGreater(
                                dark_ratio,
                                0.025,
                                "top QR is missing from the first-page upper-left area",
                            )

                    if PDFINFO:
                        info = subprocess.run(
                            [PDFINFO, str(pdf)],
                            check=True,
                            capture_output=True,
                            text=True,
                        ).stdout
                        match = re.search(r"^Pages:\s*(\d+)", info, re.MULTILINE)
                        self.assertIsNotNone(match, info)
                        self.assertGreater(int(match.group(1)), 0)


if __name__ == "__main__":
    unittest.main()
