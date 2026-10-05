"""Regenerate the checked-in visual fixture with an installed wheel and Poppler."""
from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

import serpentype

FIXTURES = Path(__file__).parent / "fixtures"
fonts = serpentype.FontRegistry()
fonts.register_file(serpentype.bundled_font_path(), "Noto Sans")
fonts.register_file(serpentype.bundled_font_path(700), "Noto Sans", 700)
fonts.register_file(str(FIXTURES / "NotoEmoji-VF.ttf"), "Noto Emoji")
for name, pages in (
    ("visual_release_reference", (1,)),
    ("visual_layout_reference", (1,)),
    ("visual_resources_reference", (1,)),
    ("visual_table_reference", (2,)),
    ("visual_page_selector_reference", (1, 2, 3, 4)),
    ("visual_typography_reference", (1,)),
    ("visual_fonts_reference", (1,)),
    ("visual_table_features_reference", (1,)),
    ("visual_webfont_reference", (1,)),
    ("visual_pagination_reference", (1, 2)),
):
    html = (FIXTURES / f"{name}.html").read_text()
    css = (FIXTURES / f"{name}.css").read_text()
    renderer = serpentype.Renderer(
        fonts=fonts, base_dir=str(FIXTURES), source_name=f"{name}.html"
    )
    pdf = bytes(renderer.layout(html, css).to_pdf())
    with tempfile.TemporaryDirectory() as directory:
        for page in pages:
            path = Path(directory) / "reference"
            subprocess.run(
                ["pdftoppm", "-f", str(page), "-singlefile", "-png", "-r", "96", "-", str(path)],
                input=pdf, check=True,
            )
            snapshot = f"{name}-page-{page}.png" if page > 1 else f"{name}.png"
            (FIXTURES / snapshot).write_bytes(path.with_suffix(".png").read_bytes())
