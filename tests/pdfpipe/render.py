"""Render local GeneralNotice templates and appendix with serpentype."""

import argparse
import base64
import re
import sys
from io import BytesIO
from pathlib import Path

import qrcode
from jinja2 import Template


PIPELINE_DIR = Path(__file__).resolve().parent
REPO_ROOT = PIPELINE_DIR.parents[1]
sys.path.insert(0, str(REPO_ROOT))

from serpentype import HTML

from cases.general_notice import GeneralNoticeCase


APPENDICES_DIR = PIPELINE_DIR / "appendices"
ASSETS_DIR = APPENDICES_DIR / "assets"
LANGUAGES = ("ru", "kk")


def generate_qr_image(payload: str, size: int = 160) -> str:
    qr = qrcode.QRCode(box_size=6, border=0)
    qr.add_data(payload)
    qr.make(fit=True)
    image = qr.make_image(fill_color="black", back_color="white").resize((size, size))
    buffer = BytesIO()
    image.save(buffer, format="PNG")
    return base64.b64encode(buffer.getvalue()).decode("ascii")


def generate_qr_html(payload: str, language: str) -> str:
    encoded = generate_qr_image(payload, size=240)
    return (
        '<div class="wrapper"><div class="row"><div class="qr">'
        f'<img src="data:image/png;base64,{encoded}" alt="QR {language}">'
        "</div></div></div>"
    )


def _page_css() -> str:
    css = (APPENDICES_DIR / "page_definition.css").read_text(encoding="utf-8")
    return (
        css.replace("__PAGES_WATERMARK_URL__", (ASSETS_DIR / "pages_watermark.jpeg").as_uri())
        .replace("__LANDSCAPE_WATERMARK_URL__", (ASSETS_DIR / "pages_watermark_landscape.jpeg").as_uri())
        .replace("__FIRST_PAGE_WATERMARK_URL__", (ASSETS_DIR / "first_page_watermark.jpeg").as_uri())
    )


def _assemble_html(language: str) -> str:
    main_template = (PIPELINE_DIR / f"{language}.html").read_text(encoding="utf-8")
    appendix_template = (APPENDICES_DIR / f"appendencies_{language}.html.jinja").read_text(
        encoding="utf-8"
    )

    qr_link = generate_qr_image(f"serpentype-local-test-document:{language}")
    main_template = re.sub(
        r'(<img[^>]*?)src="[^"]*"([^>]*alt="QR Code"[^>]*>)',
        rf'\1src="data:image/png;base64,{qr_link}"\2',
        main_template,
    )
    context = GeneralNoticeCase.get_document_dict(
        xml_qr_ru=generate_qr_html("serpentype-local-test-xml:ru", "ru"),
        xml_qr_kk=generate_qr_html("serpentype-local-test-xml:kk", "kk"),
    )[language]

    # Match worker order: main template, generated XML QR block, then appendix.
    html_template = main_template + "{{ signed_xml_qr_codes }}" + appendix_template
    html_template = html_template.translate(str.maketrans("", "", "\u200b\u200c\u200d\ufeff"))
    html_template = html_template.replace("word-break:break-all;", "")
    css_tag = f"<style>{_page_css()}</style>"
    head_end = html_template.lower().rfind("</head>")
    if head_end >= 0:
        html_template = html_template[:head_end] + css_tag + html_template[head_end:]
    else:
        html_template = css_tag + html_template

    return Template(html_template).render(**context)


def render(language: str, output_path: Path | None = None) -> Path:
    rendered_html = _assemble_html(language)
    pdf_bytes = HTML(string=rendered_html, base_url=str(PIPELINE_DIR)).write_pdf(
        presentational_hints=True,
    )
    if output_path is None:
        output_path = PIPELINE_DIR / "output" / f"{language}.pdf"
    output_path = output_path.resolve()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(pdf_bytes)
    return output_path


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Render the local GeneralNotice template and appendix with serpentype"
    )
    parser.add_argument("language", choices=(*LANGUAGES, "both"), nargs="?", default="both")
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=PIPELINE_DIR / "output",
        help="PDF output directory (default: tests/pdfpipe/output)",
    )
    args = parser.parse_args()
    languages = LANGUAGES if args.language == "both" else (args.language,)
    for language in languages:
        print(render(language, args.output_dir / f"{language}.pdf"))


if __name__ == "__main__":
    main()
