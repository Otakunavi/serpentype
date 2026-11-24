"""Compare the dev notice PDF pipeline in Serpentype and WeasyPrint."""

import argparse
from importlib import import_module
from io import StringIO
import json
from pathlib import Path
import re

from jinja2 import Template

from cases.general_notice import GeneralNoticeCase


PIPELINE_DIR = Path(__file__).resolve().parent
APPENDICES_DIR = PIPELINE_DIR / "appendices"
ASSETS_DIR = APPENDICES_DIR / "assets"
DEFAULT_FONTS_CSS = PIPELINE_DIR / "fonts" / "fonts.css"
LANGUAGES = ("ru", "kk")
MODES = ("draft", "signed")
RENDERERS = ("serpentype", "weasyprint")
TOP_QR = re.compile(r'(<img[^>]*?)src="[^"]*"([^>]*alt="QR Code"[^>]*>)')


def _assemble_html(language: str, case: dict, qr: dict) -> str:
    template = (PIPELINE_DIR / f"{language}.html").read_text(encoding="utf-8")
    logo = (ASSETS_DIR / "watermark_logo.svg").as_uri()
    texture = (ASSETS_DIR / "watermark_texture_tile.jpeg").as_uri()
    page_css = (APPENDICES_DIR / "page_definition.css").read_text(encoding="utf-8")
    page_css = page_css.replace("__WATERMARK_BACKGROUND__", f'url("{logo}"), url("{texture}")')
    appendix = (APPENDICES_DIR / f"appendencies_{language}.html.jinja").read_text(encoding="utf-8")
    template = template.replace("__DEV_PAGE_DEFINITION_CSS__", page_css).replace("__DEV_APPENDIX__", appendix)

    # The dev input labels both the draft SVG and the signed PNG as image/png.
    top_qr_src = f'src="data:image/png;base64,{qr["document_qr"]}"'
    template = TOP_QR.sub(rf"\1{top_qr_src}\2", template)
    return Template(template).render(
        **case,
        signed_xml_qr_codes=qr["signed_xml_qr_codes"],
    )


def render(html: str, renderer: str, fonts_path: Path, output_path: Path) -> int:
    backend = import_module(renderer)
    font_config = import_module(f"{renderer}.text.fonts").FontConfiguration()
    css = backend.CSS(
        string=fonts_path.read_text(encoding="utf-8"),
        font_config=font_config,
        base_url=f"{fonts_path.parent.as_uri()}/",
    )
    document = backend.HTML(file_obj=StringIO(html)).render(
        stylesheets=[css],
        font_config=font_config,
        presentational_hints=True,
    )
    output_path.parent.mkdir(parents=True, exist_ok=True)
    document.write_pdf(str(output_path))
    return len(document.pages)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language", choices=(*LANGUAGES, "both"), nargs="?", default="both")
    parser.add_argument("--mode", choices=(*MODES, "both"), default="draft")
    parser.add_argument("--renderer", choices=(*RENDERERS, "both"), default="serpentype")
    parser.add_argument("--fonts-css", type=Path, default=DEFAULT_FONTS_CSS)
    parser.add_argument("--output-dir", type=Path, default=PIPELINE_DIR / "output")
    args = parser.parse_args()

    languages = LANGUAGES if args.language == "both" else (args.language,)
    modes = MODES if args.mode == "both" else (args.mode,)
    renderers = RENDERERS if args.renderer == "both" else (args.renderer,)
    cases = GeneralNoticeCase.get_document_dict()
    qr = json.loads((PIPELINE_DIR / "qr_fixtures.json").read_text(encoding="utf-8"))
    fonts = args.fonts_css.resolve()

    for mode in modes:
        for language in languages:
            html = _assemble_html(language, cases[language], qr[mode][language])
            for renderer in renderers:
                output = (args.output_dir / mode / renderer / f"{language}.pdf").resolve()
                pages = render(html, renderer, fonts, output)
                print(f"{renderer} {mode} {language}: {pages} pages  {output}", flush=True)


if __name__ == "__main__":
    main()
