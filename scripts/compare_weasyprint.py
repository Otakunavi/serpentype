#!/usr/bin/env python3
"""Compare Serpentype's HTML/CSS API with WeasyPrint using Poppler.

Run from the repository root with:
    .venv/bin/python scripts/compare_weasyprint.py

The script renders the same checked-in HTML fixtures (with companion CSS when
present, or embedded styles otherwise) with both engines,
extracts text and line boxes with Poppler's pdftotext -bbox-layout, rasterizes
pages with pdftoppm, and writes a versioned JSON report.
"""

from __future__ import annotations

import difflib
import hashlib
from html import escape
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
VISUAL_REPORT = OUTPUT.with_name("visual-diff-report.html")
VISUAL_ASSETS = OUTPUT.parent / "visual-diff-assets"
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
    "inline_block_width_notice",
    "inline_block_width_appendix",
    "notice_ru",
    "notice_kk",
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


def find_pdftoppm(pdftotext: str) -> str:
    direct = shutil.which("pdftoppm")
    if direct:
        return direct
    sibling = Path(pdftotext).resolve().with_name("pdftoppm")
    if sibling.is_file():
        return str(sibling)
    raise RuntimeError("Poppler pdftoppm is required; install Poppler and add it to PATH")


def poppler_version(binary: str) -> str:
    result = subprocess.run([binary, "-v"], capture_output=True, text=True, check=False)
    text = result.stderr + result.stdout
    match = re.search(r"pdftotext version ([^\s]+)", text)
    return match.group(1) if match else text.splitlines()[0]


def local_resource_hashes(html_path: Path, css_path: Path | None) -> dict[str, str]:
    sources = html_path.read_text(encoding="utf-8")
    if css_path is not None:
        sources += "\n" + css_path.read_text(encoding="utf-8")
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


def raster_pages(pdf: bytes, pdftoppm: str, directory: Path, prefix: str) -> list[Path]:
    output = directory / prefix
    subprocess.run(
        [pdftoppm, "-png", "-r", "96", "-", str(output)],
        input=pdf,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=True,
    )
    return sorted(
        directory.glob(f"{prefix}-*.png"),
        key=lambda path: int(path.stem.rsplit("-", 1)[1]),
    )


def align_raster_pages(left_path: Path, right_path: Path):
    from PIL import Image, ImageChops

    with Image.open(left_path) as left_image, Image.open(right_path) as right_image:
        left = left_image.convert("RGB")
        right = right_image.convert("RGB")
    width = max(left.width, right.width)
    height = max(left.height, right.height)
    canvas_left = Image.new("RGB", (width, height), "white")
    canvas_right = Image.new("RGB", (width, height), "white")
    left_offset = ((width - left.width) // 2, (height - left.height) // 2)
    right_offset = ((width - right.width) // 2, (height - right.height) // 2)
    canvas_left.paste(left, left_offset)
    canvas_right.paste(right, right_offset)
    difference = ImageChops.difference(canvas_left, canvas_right).convert("L")
    return canvas_left, canvas_right, difference, left_offset, right_offset


def differing_regions(difference, threshold: int = 8, max_regions: int = 30) -> list[dict]:
    """Group nearby changed pixels into readable crop regions using scanline spans."""
    width, height = difference.size
    pixels = difference.tobytes()
    parents: list[int] = []
    spans: list[tuple[int, int, int, int]] = []
    previous: list[tuple[int, int, int]] = []

    def find(item: int) -> int:
        while parents[item] != item:
            parents[item] = parents[parents[item]]
            item = parents[item]
        return item

    def union(left: int, right: int) -> None:
        left_root, right_root = find(left), find(right)
        if left_root != right_root:
            parents[right_root] = left_root

    for y in range(height):
        row = pixels[y * width:(y + 1) * width]
        row_spans = []
        x = 0
        while x < width:
            if row[x] <= threshold:
                x += 1
                continue
            start = x
            x += 1
            while x < width and row[x] > threshold:
                x += 1
            end = x - 1
            if row_spans and start - row_spans[-1][1] <= 5:
                old_start, _, old_count = row_spans[-1]
                row_spans[-1] = (old_start, end, old_count + end - start + 1)
            else:
                row_spans.append((start, end, end - start + 1))

        current = []
        previous_index = 0
        for start, end, count in row_spans:
            node = len(parents)
            parents.append(node)
            spans.append((start, end, y, count))
            while previous_index < len(previous) and previous[previous_index][1] < start - 3:
                previous_index += 1
            scan_index = previous_index
            while scan_index < len(previous) and previous[scan_index][0] <= end + 3:
                union(node, previous[scan_index][2])
                scan_index += 1
            current.append((start, end, node))
        previous = current

    components: dict[int, dict] = {}
    for node, (start, end, y, count) in enumerate(spans):
        root = find(node)
        box = components.setdefault(root, {
            "left": start, "top": y, "right": end + 1, "bottom": y + 1,
            "changed_pixels": 0,
        })
        box["left"] = min(box["left"], start)
        box["top"] = min(box["top"], y)
        box["right"] = max(box["right"], end + 1)
        box["bottom"] = max(box["bottom"], y + 1)
        box["changed_pixels"] += count

    regions = [
        box for box in components.values()
        if box["changed_pixels"] >= 18
        and (box["right"] - box["left"]) * (box["bottom"] - box["top"]) >= 36
    ]
    # Combine neighboring glyph/line components into useful snippets.
    merged = True
    while merged:
        merged = False
        regions.sort(key=lambda box: (box["top"], box["left"]))
        output = []
        while regions:
            current = regions.pop(0)
            index = 0
            while index < len(regions):
                candidate = regions[index]
                if (candidate["left"] <= current["right"] + 12
                        and candidate["right"] >= current["left"] - 12
                        and candidate["top"] <= current["bottom"] + 10
                        and candidate["bottom"] >= current["top"] - 10):
                    current["left"] = min(current["left"], candidate["left"])
                    current["top"] = min(current["top"], candidate["top"])
                    current["right"] = max(current["right"], candidate["right"])
                    current["bottom"] = max(current["bottom"], candidate["bottom"])
                    current["changed_pixels"] += candidate["changed_pixels"]
                    regions.pop(index)
                    merged = True
                else:
                    index += 1
            output.append(current)
        regions = output

    regions.sort(key=lambda box: box["changed_pixels"], reverse=True)
    return regions[:max_regions]


def nearby_text(page: dict | None, box: tuple[float, float, float, float], page_offset: tuple[int, int]) -> list[str]:
    if page is None:
        return []
    left, top, right, bottom = box
    offset_x, offset_y = page_offset
    # 96 DPI raster pixels map to 0.75 PDF points.
    bounds = ((left - offset_x) * 0.75, (top - offset_y) * 0.75,
              (right - offset_x) * 0.75, (bottom - offset_y) * 0.75)
    matches = []
    for line in page["lines"]:
        x0, y0, x1, y1 = line["bbox"]
        if x1 >= bounds[0] - 8 and x0 <= bounds[2] + 8 and y1 >= bounds[1] - 8 and y0 <= bounds[3] + 8:
            matches.append(line["text"])
    return matches[:5]


def save_region_triptych(left, right, difference, box: tuple[int, int, int, int], path: Path) -> None:
    from PIL import Image, ImageOps

    pad = 12
    x0, y0, x1, y1 = box
    bounds = (max(0, x0 - pad), max(0, y0 - pad),
              min(left.width, x1 + pad), min(left.height, y1 + pad))
    left_crop = left.crop(bounds)
    right_crop = right.crop(bounds)
    diff_crop = difference.crop(bounds)
    heatmap = ImageOps.colorize(diff_crop, black="#fff7ed", white="#dc2626")
    gutter = 8
    width = left_crop.width * 3 + gutter * 2
    height = max(left_crop.height, right_crop.height, heatmap.height)
    triptych = Image.new("RGB", (width, height), "#e5e7eb")
    triptych.paste(left_crop, (0, 0))
    triptych.paste(right_crop, (left_crop.width + gutter, 0))
    triptych.paste(heatmap, (left_crop.width * 2 + gutter * 2, 0))
    path.parent.mkdir(parents=True, exist_ok=True)
    triptych.save(path, format="PNG", optimize=True)


def visual_page_comparison(
    name: str, page_number: int, left_path: Path, right_path: Path,
    native_page: dict | None, weasy_page: dict | None, assets_dir: Path,
) -> dict:
    left, right, difference, left_offset, right_offset = align_raster_pages(left_path, right_path)
    histogram = difference.histogram()
    width, height = difference.size
    pixels = width * height
    mean_delta = sum(value * count for value, count in enumerate(histogram)) / pixels
    # Use the changed-pixel mask to find localized crops for the HTML report.
    regions = differing_regions(difference)
    region_reports = []
    safe_name = re.sub(r"[^A-Za-z0-9_-]+", "_", name)
    for region_index, region in enumerate(regions, start=1):
        box = (region["left"], region["top"], region["right"], region["bottom"])
        filename = f"{safe_name}-page-{page_number:02d}-region-{region_index:02d}.png"
        image_path = assets_dir / filename
        save_region_triptych(left, right, difference, box, image_path)
        native_context = nearby_text(native_page, box, left_offset)
        weasy_context = nearby_text(weasy_page, box, right_offset)
        context_text = {
            "serpentype": native_context,
            "weasyprint": weasy_context,
        }
        region_reports.append({
            "region": region_index,
            "bbox_px": list(box),
            "changed_pixels": region["changed_pixels"],
            "image": f"visual-diff-assets/{filename}",
            "context_text": context_text,
            "description": (
                "Текст в этой области различается."
                if native_context != weasy_context
                else "Текст поблизости совпадает; различается его положение или отрисовка."
            ),
        })
    return {
        "serpentype_size_px": [left.width, left.height],
        "weasyprint_size_px": [right.width, right.height],
        "comparison_canvas_px": [left.width, left.height],
        "mean_grayscale_delta_8bit": mean_delta,
        "changed_pixel_fraction_over_8": sum(histogram[9:]) / pixels,
        "changed_pixel_fraction_over_32": sum(histogram[33:]) / pixels,
        "exact_pixel_fraction": histogram[0] / pixels,
        "within_tolerance": mean_delta <= 3.01 and sum(histogram[9:]) / pixels <= 0.05,
        "difference_regions": region_reports,
    }


def visual_comparison(
    name: str, native_pdf: bytes, weasy_pdf: bytes, pdftoppm: str,
    native_text_pages: list[dict], weasy_text_pages: list[dict], assets_dir: Path,
) -> dict:
    import tempfile

    try:
        with tempfile.TemporaryDirectory(prefix="serpentype-visual-") as temp_dir:
            directory = Path(temp_dir)
            native_pages = raster_pages(native_pdf, pdftoppm, directory, "serpentype")
            weasy_pages = raster_pages(weasy_pdf, pdftoppm, directory, "weasyprint")
            comparisons = []
            for index in range(max(len(native_pages), len(weasy_pages))):
                native_page = native_pages[index] if index < len(native_pages) else None
                weasy_page = weasy_pages[index] if index < len(weasy_pages) else None
                page = {
                    "page": index + 1,
                    "present": {"serpentype": native_page is not None, "weasyprint": weasy_page is not None},
                }
                if native_page is not None and weasy_page is not None:
                    page.update(visual_page_comparison(
                        name, index + 1, native_page, weasy_page,
                        native_text_pages[index] if index < len(native_text_pages) else None,
                        weasy_text_pages[index] if index < len(weasy_text_pages) else None,
                        assets_dir,
                    ))
                else:
                    page["within_tolerance"] = False
                comparisons.append(page)
            measured = [page for page in comparisons if "mean_grayscale_delta_8bit" in page]
            return {
                "status": "compared",
                "dpi": 96,
                "alignment": "page images centered on a white canvas sized to the larger page",
                "tolerance": {"mean_grayscale_delta_8bit_max": 3.01, "changed_pixel_fraction_over_8_max": 0.05},
                "page_count": {"serpentype": len(native_pages), "weasyprint": len(weasy_pages)},
                "pages": comparisons,
                "tolerance_pass_page_count": sum(page["within_tolerance"] for page in comparisons),
                "compared_page_count": len(measured),
            }
    except Exception as error:
        return {"status": "error", "error": f"{type(error).__name__}: {error}"}


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


def build_visual_html(results: dict) -> str:
    entries = []
    total_regions = 0
    for name, result in results.items():
        visual = result.get("visual", {})
        if visual.get("status") != "compared":
            continue
        pages = visual.get("pages", [])
        document_regions = [
            (page, region)
            for page in pages
            for region in page.get("difference_regions", [])
        ]
        total_regions += len(document_regions)
        if not document_regions:
            entries.append(
                f'<section class="document"><h2>{escape(name)}</h2>'
                '<p class="ok">No localized differences above the crop threshold.</p></section>'
            )
            continue

        page_pass = visual.get("tolerance_pass_page_count", 0)
        page_total = visual.get("compared_page_count", 0)
        cards = []
        for page, region in document_regions:
            left, top, right, bottom = region["bbox_px"]
            width, height = right - left, bottom - top
            contexts = region.get("context_text", {})
            native_text = "\n".join(contexts.get("serpentype", [])) or "No nearby text extracted"
            weasy_text = "\n".join(contexts.get("weasyprint", [])) or "No nearby text extracted"
            cards.append(
                '<article class="region">'
                f'<h3>Page {page["page"]}, region {region["region"]}</h3>'
                f'<p>{escape(region["description"])} Координаты: x={left}, y={top}; '
                f'{width}×{height} px; {region["changed_pixels"]} changed pixels.</p>'
                '<div class="legend"><span>Serpentype</span><span>WeasyPrint</span>'
                '<span>Difference heatmap</span></div>'
                f'<img src="{escape(region["image"], quote=True)}" '
                f'alt="Serpentype and WeasyPrint crops and difference heatmap, page {page["page"]}, region {region["region"]}">'
                '<details><summary>Nearby extracted text</summary>'
                '<div class="text-context"><div><strong>Serpentype</strong>'
                f'<pre>{escape(native_text)}</pre></div><div><strong>WeasyPrint</strong>'
                f'<pre>{escape(weasy_text)}</pre></div></div></details>'
                '</article>'
            )
        entries.append(
            f'<section class="document"><h2>{escape(name)}</h2>'
            f'<p class="summary">{len(document_regions)} difference regions; '
            f'{page_pass}/{page_total} pages meet the page visual tolerance.</p>'
            + "\n".join(cards)
            + '</section>'
        )

    return """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Serpentype / WeasyPrint visual differences</title>
<style>
body{font:16px/1.5 system-ui,sans-serif;color:#182230;background:#f4f6f8;margin:0;padding:2rem}
main{max-width:1200px;margin:auto}h1{margin-bottom:.25rem}.intro{color:#52606d;margin-top:0}
.document{background:#fff;border:1px solid #dce2e8;border-radius:10px;padding:1.25rem;margin:1.5rem 0}
.summary{color:#52606d}.ok{color:#18794e}.region{border-top:1px solid #e6eaee;padding:1rem 0}
.region img{display:block;width:100%;height:auto;border:1px solid #cbd2d9;background:white}
.legend{display:grid;grid-template-columns:repeat(3,1fr);font-size:.85rem;font-weight:600;color:#52606d;margin:.5rem 0}
.text-context{display:grid;grid-template-columns:1fr 1fr;gap:1rem}.text-context pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f4f6f8;padding:.75rem;border-radius:6px}
details{margin-top:.65rem}footer{color:#52606d;font-size:.9rem;margin-top:2rem}
@media(max-width:700px){body{padding:1rem}.text-context{grid-template-columns:1fr}}
</style></head><body><main>
<h1>Visual difference report</h1>
<p class="intro">Cropped regions only. Each image shows the Serpentype crop, the WeasyPrint crop, and an amplified difference heatmap. Crop threshold: grayscale difference over 8; pages rendered at 96 DPI.</p>
""" + "\n".join(entries) + f"\n<footer>{total_regions} localized regions across {len(results)} fixtures. Images are stored beside this HTML report in visual-diff-assets/.</footer>\n</main></body></html>\n"


def render_pair(name: str, pdftotext: str, pdftoppm: str) -> dict:
    html_path = FIXTURES / f"{name}.html"
    css_path = FIXTURES / f"{name}.css"
    if not css_path.is_file():
        css_path = None
    html_text = html_path.read_text(encoding="utf-8")
    css_text = css_path.read_text(encoding="utf-8") if css_path is not None else ""
    fixture_base = FIXTURES.as_uri() + "/"
    css_sha256 = sha256(css_path.read_bytes()) if css_path is not None else None

    native_config = FontConfiguration()
    font_css = register_common_fonts(native_config)
    native_css = CSS(
        string=font_css + css_text,
        base_url=fixture_base,
        font_config=native_config,
    )
    native_pdf = None
    render_errors = {}
    try:
        native_pdf = HTML(string=html_text, base_url=fixture_base).write_pdf(
            stylesheets=[native_css], font_config=native_config
        )
    except Exception as error:
        render_errors["serpentype"] = f"{type(error).__name__}: {error}"

    weasy_config = WeasyFontConfiguration()
    weasy_css = WeasyCSS(
        string=font_css + css_text,
        base_url=fixture_base,
        font_config=weasy_config,
    )
    weasy_pdf = None
    try:
        weasy_pdf = WeasyHTML(string=html_text, base_url=fixture_base).write_pdf(
            stylesheets=[weasy_css], font_config=weasy_config
        )
    except Exception as error:
        render_errors["weasyprint"] = f"{type(error).__name__}: {error}"

    if render_errors:
        return {
            "status": "render_error",
            "source": {
                "html_sha256": sha256(html_path.read_bytes()),
                "css_sha256": css_sha256,
            },
            "resources": local_resource_hashes(html_path, css_path),
            "render_errors": render_errors,
            "rendered": {
                "serpentype": native_pdf is not None,
                "weasyprint": weasy_pdf is not None,
            },
        }

    project_pages = pdf_text_layout(native_pdf, pdftotext)
    weasy_pages = pdf_text_layout(weasy_pdf, pdftotext)
    visual = visual_comparison(
        name, native_pdf, weasy_pdf, pdftoppm, project_pages, weasy_pages, VISUAL_ASSETS
    )
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
        "status": "compared",
        "source": {
            "html_sha256": sha256(html_path.read_bytes()),
            "css_sha256": css_sha256,
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
        "visual": visual,
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
    pdftoppm = find_pdftoppm(pdftotext)
    results = {name: render_pair(name, pdftotext, pdftoppm) for name in DOCUMENTS}
    compared = [result for result in results.values() if result["status"] == "compared"]
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
            "pdftoppm_path": pdftoppm,
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
            "raster_comparison": {
                "renderer": "Poppler pdftoppm",
                "dpi": 96,
                "pixel_alignment": "centered on a white canvas sized to the larger page; no scaling",
                "grayscale_delta": "Pillow ImageChops difference converted to 8-bit grayscale",
                "tolerance": {
                    "mean_grayscale_delta_8bit_max": 3.01,
                    "changed_pixel_fraction_over_8_max": 0.05,
                },
                "difference_crops": {
                    "pixel_threshold_grayscale_over": 8,
                    "minimum_changed_pixels": 18,
                    "maximum_regions_per_page": 30,
                    "crop_padding_px": 12,
                    "report_html": str(VISUAL_REPORT.relative_to(ROOT)),
                    "assets_directory": str(VISUAL_ASSETS.relative_to(ROOT)),
                },
            },
        },
        "documents": results,
        "aggregate": {
            "document_count": len(results),
            "compared_document_count": len(compared),
            "render_error_document_count": len(results) - len(compared),
            "total_pages_serpentype": sum(result["pages"]["serpentype"] for result in compared),
            "total_pages_weasyprint": sum(result["pages"]["weasyprint"] for result in compared),
            "total_words_serpentype": sum(result["text"]["serpentype_words"] for result in compared),
            "total_words_weasyprint": sum(result["text"]["weasyprint_words"] for result in compared),
            "total_lines_serpentype": sum(result["text"]["serpentype_lines"] for result in compared),
            "total_lines_weasyprint": sum(result["text"]["weasyprint_lines"] for result in compared),
            "page_count_matches": sum(result["pages"]["count_match"] for result in compared),
            "mean_word_sequence_similarity_pct": sum(
                result["text"]["word_sequence_similarity_pct"] for result in compared
            ) / len(compared),
            "mean_line_sequence_similarity_pct": sum(
                result["text"]["line_sequence_similarity_pct"] for result in compared
            ) / len(compared),
            "exact_line_sequence_document_count": sum(
                result["text"]["exact_line_sequence"] for result in compared
            ),
            "matched_line_count": sum(result["matched_lines"] for result in compared),
            "mean_bbox_delta_of_document_means_pt": sum(
                result["line_bbox_centered_delta_pt"]["mean"]
                for result in compared if result["line_bbox_centered_delta_pt"]
            ) / max(1, sum(bool(result["line_bbox_centered_delta_pt"]) for result in compared)),
            "all_documents_exact_line_sequence": all(
                result["text"]["exact_line_sequence"] for result in compared
            ),
            "visual_comparison_document_count": sum(
                result.get("visual", {}).get("status") == "compared" for result in compared
            ),
            "visual_render_error_document_count": sum(
                result.get("visual", {}).get("status") == "error" for result in compared
            ),
            "visual_compared_page_count": sum(
                result.get("visual", {}).get("compared_page_count", 0) for result in compared
            ),
            "visual_tolerance_pass_page_count": sum(
                result.get("visual", {}).get("tolerance_pass_page_count", 0) for result in compared
            ),
            "visual_tolerance_fail_page_count": sum(
                len(result.get("visual", {}).get("pages", []))
                - result.get("visual", {}).get("tolerance_pass_page_count", 0)
                for result in compared if result.get("visual", {}).get("status") == "compared"
            ),
        },
    }
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    VISUAL_REPORT.write_text(build_visual_html(results), encoding="utf-8")
    report["visual_report"] = str(VISUAL_REPORT.relative_to(ROOT))
    OUTPUT.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({
        "output": str(OUTPUT.relative_to(ROOT)),
        "visual_report": str(VISUAL_REPORT.relative_to(ROOT)),
        **report["aggregate"],
    }, indent=2))


if __name__ == "__main__":
    main()
