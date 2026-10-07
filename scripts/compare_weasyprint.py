#!/usr/bin/env python3
"""Compare Serpentype's HTML/CSS API with WeasyPrint using Poppler text boxes.

Run from the repository root with:
    .venv/bin/python scripts/compare_weasyprint.py

The script renders the same checked-in HTML/CSS fixtures with both engines,
extracts text and line boxes with Poppler's pdftotext -bbox-layout, and writes a
versioned JSON report. It does not rasterize pages or compare pixels.
"""

from __future__ import annotations

import difflib
import hashlib
import json
import platform
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
from datetime import datetime, timezone
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from urllib.parse import unquote, urlsplit

import weasyprint
from weasyprint import CSS as WeasyCSS
from weasyprint import HTML as WeasyHTML
from weasyprint.text.fonts import FontConfiguration as WeasyFontConfiguration

import serpentype
from serpentype import CSS, HTML, bundled_font_path
from serpentype.text.fonts import FontConfiguration


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests" / "fixtures"
OUTPUT = ROOT / "output" / "compatibility" / "weasyprint-comparison.json"
DOCUMENTS = (
    "visual_release_reference",
    "visual_layout_reference",
    "visual_resources_reference",
    "visual_table_reference",
    "visual_table_features_reference",
    "visual_page_selector_reference",
    "visual_typography_reference",
    "visual_fonts_reference",
    "visual_webfont_reference",
    "visual_pagination_reference",
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def find_pdftotext() -> str:
    direct = shutil.which("pdftotext")
    if direct:
        return direct
    pdftoppm = shutil.which("pdftoppm")
    if pdftoppm:
        path = Path(pdftoppm).resolve()
        candidates = [path.with_name("pdftotext")]
        if len(path.parents) > 2:
            candidates.extend((
                path.parents[2] / "native/poppler/bin/pdftotext",
                path.parents[2] / "native/poppler/poppler/bin/pdftotext",
            ))
        for candidate in candidates:
            if candidate.is_file():
                return str(candidate)
    raise RuntimeError("Poppler pdftotext is required; install Poppler and add it to PATH")


def poppler_version(binary: str) -> str:
    result = subprocess.run([binary, "-v"], capture_output=True, text=True, check=False)
    text = result.stderr + result.stdout
    match = re.search(r"pdftotext version ([^\s]+)", text)
    return match.group(1) if match else text.splitlines()[0]


def local_resource_hashes(html_path: Path, css_path: Path) -> dict[str, str]:
    sources = html_path.read_text(encoding="utf-8") + "\n" + css_path.read_text(encoding="utf-8")
    references = re.findall(
        r"(?:src|href)\s*=\s*(['\"])(.*?)\1|url\(\s*(['\"]?)(.*?)\3\s*\)",
        sources,
        re.IGNORECASE | re.DOTALL,
    )
    hashes = {}
    for reference in references:
        value = reference[1] or reference[3]
        parsed = urlsplit(value)
        if parsed.scheme not in ("", "file"):
            continue
        resource = Path(unquote(parsed.path))
        if not resource.is_absolute():
            resource = FIXTURES / resource
        if resource.is_file():
            try:
                name = resource.resolve().relative_to(FIXTURES.resolve()).as_posix()
            except ValueError:
                continue
            hashes[name] = sha256(resource.read_bytes())
    return dict(sorted(hashes.items()))


def poppler_object_inventory(pdf: bytes, pdftotext: str) -> dict:
    directory = Path(pdftotext).resolve().parent
    inventory = {}
    for name, command, arguments in (
        ("fonts", "pdffonts", []),
        ("images", "pdfimages", ["-list"]),
    ):
        binary = directory / command
        if not binary.is_file():
            inventory[name] = {"available": False}
            continue
        result = subprocess.run(
            [str(binary), *arguments, "-"],
            input=pdf,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=True,
        )
        rows = result.stdout.decode("utf-8", errors="replace").splitlines()
        if name == "fonts":
            header = next((index for index, row in enumerate(rows) if row.lstrip().startswith("name ")), None)
            data_rows = [row.strip() for row in rows[header + 2:] if row.strip()] if header is not None else []
        else:
            data_rows = [row.strip() for row in rows if re.match(r"^\s*\d+\s+", row)]
        inventory[name] = {"available": True, "rows": data_rows, "count": len(data_rows)}
    return inventory


def pdf_text_layout(pdf: bytes, pdftotext: str) -> list[dict]:
    result = subprocess.run(
        [pdftotext, "-bbox-layout", "-", "-"],
        input=pdf,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    root = ET.fromstring(result.stdout)
    ns = "{http://www.w3.org/1999/xhtml}"
    pages = []
    for page in root.findall(".//" + ns + "page"):
        lines = []
        words = []
        for line in page.findall(".//" + ns + "line"):
            line_words = line.findall(ns + "word")
            line_text = " ".join("".join(word.itertext()) for word in line_words)
            if not line_text:
                continue
            lines.append({
                "text": line_text,
                "bbox": [float(line.get(key)) for key in ("xMin", "yMin", "xMax", "yMax")],
            })
            words.extend("".join(word.itertext()) for word in line_words)
        pages.append({
            "width_pt": float(page.get("width")),
            "height_pt": float(page.get("height")),
            "lines": lines,
            "words": words,
        })
    return pages


def similarity(left: list[str], right: list[str]) -> float:
    if not left and not right:
        return 1.0
    return difflib.SequenceMatcher(None, left, right, autojunk=False).ratio()


def comparable_text(value: str) -> str:
    """Normalize invisible soft hyphens while retaining the selected glyph."""
    return value.replace("\u00ad", "")


def comparable_sequence(values: list[str]) -> list[str]:
    return [comparable_text(value) for value in values]


def bbox_deltas(project_pages: list[dict], weasy_pages: list[dict]) -> tuple[list[float], int]:
    deltas = []
    matched_lines = 0
    for project_page, weasy_page in zip(project_pages, weasy_pages):
        page_deltas, page_matches = line_bbox_deltas(project_page, weasy_page)
        deltas.extend(page_deltas)
        matched_lines += page_matches
    return deltas, matched_lines


def line_bbox_deltas(project_page: dict, weasy_page: dict) -> tuple[list[float], int]:
    deltas = []
    project_lines = project_page["lines"]
    weasy_lines = weasy_page["lines"]
    matcher = difflib.SequenceMatcher(
        None,
        comparable_sequence([line["text"] for line in project_lines]),
        comparable_sequence([line["text"] for line in weasy_lines]),
        autojunk=False,
    )
    for block in matcher.get_matching_blocks():
        for offset in range(block.size):
            project_box = project_lines[block.a + offset]["bbox"]
            weasy_box = weasy_lines[block.b + offset]["bbox"]
            for index, (project_coord, weasy_coord) in enumerate(zip(project_box, weasy_box)):
                project_extent = project_page["width_pt"] if index in (0, 2) else project_page["height_pt"]
                weasy_extent = weasy_page["width_pt"] if index in (0, 2) else weasy_page["height_pt"]
                deltas.append(abs(
                    (project_coord - project_extent / 2)
                    - (weasy_coord - weasy_extent / 2)
                ))
    return deltas, sum(block.size for block in matcher.get_matching_blocks())


def sequence_differences(left: list[str], right: list[str], limit: int = 50) -> list[dict]:
    matcher = difflib.SequenceMatcher(None, left, right, autojunk=False)
    differences = []
    for tag, left_start, left_end, right_start, right_end in matcher.get_opcodes():
        if tag == "equal":
            continue
        differences.append({
            "operation": tag,
            "serpentype_index_range": [left_start, left_end],
            "weasyprint_index_range": [right_start, right_end],
            "serpentype": left[left_start:left_end],
            "weasyprint": right[right_start:right_end],
        })
        if len(differences) == limit:
            break
    return differences


def distribution(values: list[float]) -> dict | None:
    if not values:
        return None
    ordered = sorted(values)
    p95_index = min(len(ordered) - 1, int(0.95 * (len(ordered) - 1)))
    return {
        "mean": sum(ordered) / len(ordered),
        "median": ordered[len(ordered) // 2],
        "p95": ordered[p95_index],
        "max": ordered[-1],
        "sample_count": len(ordered),
    }


def register_common_fonts(native_config: FontConfiguration) -> str:
    regular = Path(bundled_font_path()).resolve()
    bold = Path(bundled_font_path(700)).resolve()
    emoji = (FIXTURES / "NotoEmoji-VF.ttf").resolve()
    native_config.registry.register_file(str(emoji), family="Noto Emoji")
    return (
        f"@font-face {{ font-family: 'Noto Sans'; src: url('{regular.as_uri()}'); font-weight: 400; }}\n"
        f"@font-face {{ font-family: 'Noto Sans'; src: url('{bold.as_uri()}'); font-weight: 700; }}\n"
        f"@font-face {{ font-family: 'Noto Emoji'; src: url('{emoji.as_uri()}'); }}\n"
    )


def serpentype_version() -> str:
    try:
        return version("serpentype")
    except PackageNotFoundError:
        metadata = ROOT / "pyproject.toml"
        match = re.search(r'^version\s*=\s*"([^"]+)"', metadata.read_text(), re.MULTILINE)
        return match.group(1) if match else "unknown"


def render_pair(name: str, pdftotext: str) -> dict:
    html_path = FIXTURES / f"{name}.html"
    css_path = FIXTURES / f"{name}.css"
    html_text = html_path.read_text(encoding="utf-8")
    css_text = css_path.read_text(encoding="utf-8")
    fixture_base = FIXTURES.as_uri() + "/"

    native_config = FontConfiguration()
    font_css = register_common_fonts(native_config)
    native_css = CSS(
        string=font_css + css_text,
        base_url=fixture_base,
        font_config=native_config,
    )
    native_pdf = HTML(string=html_text, base_url=fixture_base).write_pdf(
        stylesheets=[native_css], font_config=native_config
    )

    weasy_config = WeasyFontConfiguration()
    weasy_css = WeasyCSS(
        string=font_css + css_text,
        base_url=fixture_base,
        font_config=weasy_config,
    )
    weasy_pdf = WeasyHTML(string=html_text, base_url=fixture_base).write_pdf(
        stylesheets=[weasy_css], font_config=weasy_config
    )

    project_pages = pdf_text_layout(native_pdf, pdftotext)
    weasy_pages = pdf_text_layout(weasy_pdf, pdftotext)
    project_words = [word for page in project_pages for word in page["words"]]
    weasy_words = [word for page in weasy_pages for word in page["words"]]
    project_lines = [line["text"] for page in project_pages for line in page["lines"]]
    weasy_lines = [line["text"] for page in weasy_pages for line in page["lines"]]
    deltas, matched_lines = bbox_deltas(project_pages, weasy_pages)
    common_page_count = min(len(project_pages), len(weasy_pages))
    page_width_deltas = [
        abs(project_pages[index]["width_pt"] - weasy_pages[index]["width_pt"])
        for index in range(common_page_count)
    ]
    page_height_deltas = [
        abs(project_pages[index]["height_pt"] - weasy_pages[index]["height_pt"])
        for index in range(common_page_count)
    ]
    per_page = []
    for index in range(max(len(project_pages), len(weasy_pages))):
        project_page = project_pages[index] if index < len(project_pages) else None
        weasy_page = weasy_pages[index] if index < len(weasy_pages) else None
        page_delta_values = []
        page_matched_lines = 0
        if project_page is not None and weasy_page is not None:
            page_delta_values, page_matched_lines = line_bbox_deltas(project_page, weasy_page)
        page_project_lines = [line["text"] for line in project_page["lines"]] if project_page else []
        page_weasy_lines = [line["text"] for line in weasy_page["lines"]] if weasy_page else []
        comparable_project_lines = comparable_sequence(page_project_lines)
        comparable_weasy_lines = comparable_sequence(page_weasy_lines)
        project_words_page = project_page["words"] if project_page else []
        weasy_words_page = weasy_page["words"] if weasy_page else []
        per_page.append({
            "page": index + 1,
            "present": {"serpentype": project_page is not None, "weasyprint": weasy_page is not None},
            "dimensions_pt": {
                "serpentype": ({"width": project_page["width_pt"], "height": project_page["height_pt"]}
                               if project_page else None),
                "weasyprint": ({"width": weasy_page["width_pt"], "height": weasy_page["height_pt"]}
                               if weasy_page else None),
            },
            "text": {
                "serpentype_words": len(project_words_page),
                "weasyprint_words": len(weasy_words_page),
                "word_sequence_similarity_pct": 100 * similarity(
                    comparable_sequence(project_words_page),
                    comparable_sequence(weasy_words_page),
                ),
                "serpentype_lines": len(page_project_lines),
                "weasyprint_lines": len(page_weasy_lines),
                "line_sequence_similarity_pct": 100 * similarity(
                    comparable_project_lines, comparable_weasy_lines
                ),
                "exact_line_sequence": comparable_project_lines == comparable_weasy_lines,
            },
            "matched_lines": page_matched_lines,
            "line_bbox_centered_delta_pt": distribution(page_delta_values),
            "extracted_text": {
                "serpentype": {
                    "words": project_words_page,
                    "lines": page_project_lines,
                    "line_boxes": project_page["lines"] if project_page else [],
                },
                "weasyprint": {
                    "words": weasy_words_page,
                    "lines": page_weasy_lines,
                    "line_boxes": weasy_page["lines"] if weasy_page else [],
                },
            },
        })
    return {
        "source": {
            "html_sha256": sha256(html_path.read_bytes()),
            "css_sha256": sha256(css_path.read_bytes()),
        },
        "resources": local_resource_hashes(html_path, css_path),
        "outputs": {
            "serpentype": {
                "pdf_sha256": sha256(native_pdf),
                "extracted_layout_sha256": sha256(json.dumps(
                    project_pages, ensure_ascii=False, sort_keys=True,
                    separators=(",", ":"),
                ).encode("utf-8")),
                "bytes": len(native_pdf),
                "poppler_objects": poppler_object_inventory(native_pdf, pdftotext),
            },
            "weasyprint": {
                "pdf_sha256": sha256(weasy_pdf),
                "extracted_layout_sha256": sha256(json.dumps(
                    weasy_pages, ensure_ascii=False, sort_keys=True,
                    separators=(",", ":"),
                ).encode("utf-8")),
                "bytes": len(weasy_pdf),
                "poppler_objects": poppler_object_inventory(weasy_pdf, pdftotext),
            },
        },
        "pages": {
            "serpentype": len(project_pages),
            "weasyprint": len(weasy_pages),
            "count_match": len(project_pages) == len(weasy_pages),
            "mean_width_delta_pt": sum(page_width_deltas) / len(page_width_deltas)
            if page_width_deltas else None,
            "mean_height_delta_pt": sum(page_height_deltas) / len(page_height_deltas)
            if page_height_deltas else None,
        },
        "text": {
            "serpentype_words": len(project_words),
            "weasyprint_words": len(weasy_words),
            "word_sequence_similarity_pct": 100 * similarity(
                comparable_sequence(project_words), comparable_sequence(weasy_words)
            ),
            "serpentype_lines": len(project_lines),
            "weasyprint_lines": len(weasy_lines),
            "line_sequence_similarity_pct": 100 * similarity(
                comparable_sequence(project_lines), comparable_sequence(weasy_lines)
            ),
            "exact_line_sequence": comparable_sequence(project_lines)
            == comparable_sequence(weasy_lines),
            "line_differences": sequence_differences(
                comparable_sequence(project_lines), comparable_sequence(weasy_lines)
            ),
            "word_differences": sequence_differences(
                comparable_sequence(project_words), comparable_sequence(weasy_words)
            ),
        },
        "line_bbox_centered_delta_pt": distribution(deltas),
        "matched_lines": matched_lines,
        "per_page": per_page,
        "page_boxes": [
            {
                "page": index + 1,
                "serpentype": {
                    "width_pt": project_pages[index]["width_pt"],
                    "height_pt": project_pages[index]["height_pt"],
                },
                "weasyprint": {
                    "width_pt": weasy_pages[index]["width_pt"],
                    "height_pt": weasy_pages[index]["height_pt"],
                },
            }
            for index in range(common_page_count)
        ],
    }


def main() -> None:
    pdftotext = find_pdftotext()
    results = {name: render_pair(name, pdftotext) for name in DOCUMENTS}
    regular_font = Path(bundled_font_path()).resolve()
    bold_font = Path(bundled_font_path(700)).resolve()
    emoji_font = (FIXTURES / "NotoEmoji-VF.ttf").resolve()

    report = {
        "schema_version": 1,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "environment": {
            "serpentype_version": serpentype_version(),
            "git_tag": subprocess.run(
                ["git", "describe", "--tags", "--always"],
                cwd=ROOT, capture_output=True, text=True, check=True,
            ).stdout.strip(),
            "git_dirty": bool(subprocess.run(
                ["git", "status", "--porcelain"],
                cwd=ROOT, capture_output=True, text=True, check=True,
            ).stdout.strip()),
            "python": sys.version.split()[0],
            "platform": platform.platform(),
            "weasyprint_version": weasyprint.__version__,
            "poppler_pdftotext_version": poppler_version(pdftotext),
            "pdftotext_path": pdftotext,
            "font_assets": {
                "noto_sans_regular_sha256": sha256(regular_font.read_bytes()),
                "noto_sans_bold_sha256": sha256(bold_font.read_bytes()),
                "noto_emoji_sha256": sha256(emoji_font.read_bytes()),
            },
        },
        "method": {
            "font_policy": "same bundled Noto Sans Regular/Bold and Noto Emoji faces in both renderers",
            "layout_extraction": "Poppler pdftotext -bbox-layout",
            "bbox_alignment": "text-identical lines paired in page order; coordinates centered within each page; absolute coordinate deltas measured in PDF points",
            "distribution_percentile": "p95 uses the nearest-rank lower index floor(0.95 * (n - 1))",
            "unicode_normalization": "U+00AD is treated as invisible; selected manual breaks compare by their painted hyphen glyph",
            "no_raster_comparison": True,
        },
        "documents": results,
        "aggregate": {
            "document_count": len(results),
            "total_pages_serpentype": sum(result["pages"]["serpentype"] for result in results.values()),
            "total_pages_weasyprint": sum(result["pages"]["weasyprint"] for result in results.values()),
            "total_words_serpentype": sum(result["text"]["serpentype_words"] for result in results.values()),
            "total_words_weasyprint": sum(result["text"]["weasyprint_words"] for result in results.values()),
            "total_lines_serpentype": sum(result["text"]["serpentype_lines"] for result in results.values()),
            "total_lines_weasyprint": sum(result["text"]["weasyprint_lines"] for result in results.values()),
            "page_count_matches": sum(result["pages"]["count_match"] for result in results.values()),
            "mean_word_sequence_similarity_pct": sum(
                result["text"]["word_sequence_similarity_pct"] for result in results.values()
            ) / len(results),
            "mean_line_sequence_similarity_pct": sum(
                result["text"]["line_sequence_similarity_pct"] for result in results.values()
            ) / len(results),
            "exact_line_sequence_document_count": sum(
                result["text"]["exact_line_sequence"] for result in results.values()
            ),
            "matched_line_count": sum(result["matched_lines"] for result in results.values()),
            "mean_bbox_delta_of_document_means_pt": sum(
                result["line_bbox_centered_delta_pt"]["mean"]
                for result in results.values() if result["line_bbox_centered_delta_pt"]
            ) / sum(bool(result["line_bbox_centered_delta_pt"]) for result in results.values()),
            "all_documents_exact_line_sequence": all(
                result["text"]["exact_line_sequence"] for result in results.values()
            ),
        },
    }
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(OUTPUT.relative_to(ROOT)), **report["aggregate"]}, indent=2))


if __name__ == "__main__":
    main()
