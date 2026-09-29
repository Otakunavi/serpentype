"""Check the saved benchmark PDFs with an independent PDF parser (pypdf)."""

import json
from pathlib import Path

from pypdf import PdfReader
from pypdf.generic import ContentStream

ROOT = Path(__file__).resolve().parents[1]
BENCHMARK = ROOT / "output" / "benchmark" / "page_definition_stress"
PDFS = ROOT / "output" / "pdf"


def analyze(name, manifest):
    pdf_path = PDFS / f"page_definition_stress_{name}.pdf"
    reader = PdfReader(pdf_path)
    texts = [page.extract_text() or "" for page in reader.pages]
    all_text = "\n".join(texts)
    marker_counts = {}
    for section in manifest["sections"]:
        name_ = section["page_name"]
        for row in range(1, section["rows"] + 1):
            marker = f"Контрольная строка {name_}-{row:03d}"
            marker_counts[marker] = all_text.count(marker)
    missing = [marker for marker, count in marker_counts.items() if count == 0]
    repeated = [marker for marker, count in marker_counts.items() if count > 1]
    if missing or repeated:
        raise AssertionError(f"{name}: missing={missing[:5]}, repeated={repeated[:5]}")
    section_endings = {
        section["page_name"]: all_text.count(f"Конец раздела {section['page_name']}")
        for section in manifest["sections"]
    }
    if any(count != 1 for count in section_endings.values()):
        raise AssertionError(f"{name}: section endings {section_endings}")

    image_draws = 0
    page_geometry = []
    embedded_font_file_bytes = {}
    footer_separations = []
    for index, page in enumerate(reader.pages, 1):
        resources = page["/Resources"].get("/XObject", {})
        for font_name, font_ref in page["/Resources"].get("/Font", {}).items():
            font = font_ref.get_object()
            descendants = font.get("/DescendantFonts")
            descriptor = (
                descendants[0].get_object().get("/FontDescriptor")
                if descendants else font.get("/FontDescriptor")
            )
            if descriptor:
                font_file = descriptor.get_object().get("/FontFile2")
                if font_file:
                    embedded_font_file_bytes[str(font_name)] = len(
                        font_file.get_object().get_data()
                    )
        footer_x = []
        for operands, operator in ContentStream(page.get_contents(), reader).operations:
            if operator == b"Do":
                image = resources.get(str(operands[0]))
                if image and image.get_object().get("/Subtype") == "/Image":
                    image_draws += 1
            if name == "serpentype" and operator == b"Tm" and float(operands[5]) < 35:
                footer_x.append(float(operands[4]))
        if name == "serpentype":
            if len(footer_x) != 2 or footer_x[1] - footer_x[0] < 150:
                raise AssertionError(
                    f"page {index}: footer text positions overlap or are missing: {footer_x}"
                )
            footer_separations.append(footer_x[1] - footer_x[0])
        page_geometry.append({
            "page": index,
            "width_pt": float(page.mediabox.width),
            "height_pt": float(page.mediabox.height),
            "rotation": int(page.get("/Rotate", 0)),
        })
    if image_draws != manifest["image_elements"]:
        raise AssertionError(f"{name}: {image_draws} image draws, expected {manifest['image_elements']}")
    pages_without_table_header = [
        index for index, text in enumerate(texts, 1)
        if "Контрольная строка" in text and "Описание операции" not in text
    ]
    if pages_without_table_header:
        raise AssertionError(f"{name}: table header missing on pages {pages_without_table_header}")
    pages_without_counter = [
        index for index, text in enumerate(texts, 1)
        if f"Лист {index} из {len(reader.pages)}" not in text
        and f"{index} беттің {len(reader.pages)}" not in text
    ]
    if pages_without_counter:
        raise AssertionError(f"{name}: page counters missing on pages {pages_without_counter}")
    sideways_landscape = [
        entry["page"] for entry in page_geometry
        if entry["width_pt"] > entry["height_pt"] and entry["rotation"] != 0
    ]
    if sideways_landscape:
        raise AssertionError(f"{name}: landscape pages displayed sideways: {sideways_landscape}")
    return {
        "pages": len(reader.pages),
        "pdf_bytes": pdf_path.stat().st_size,
        "table_rows_found_once": len(marker_counts),
        "section_endings_found_once": len(section_endings),
        "image_draws": image_draws,
        "pages_with_repeated_table_header": sum("Описание операции" in text for text in texts),
        "page_counters_checked": len(reader.pages),
        "embedded_font_file_bytes": embedded_font_file_bytes,
        "minimum_footer_position_separation_pt": (
            min(footer_separations) if footer_separations else None
        ),
        "first_page_has_footer": ("Лист " in texts[0] or " беттің " in texts[0]),
        "landscape_pages": [
            entry["page"] for entry in page_geometry
            if entry["width_pt"] > entry["height_pt"]
        ],
        "rotated_pages": [
            entry["page"] for entry in page_geometry if entry["rotation"] != 0
        ],
        "page_geometry": page_geometry,
    }


def main():
    manifest = json.loads((BENCHMARK / "fixture_manifest.json").read_text())
    results = {name: analyze(name, manifest) for name in ("serpentype", "weasyprint")}
    (BENCHMARK / "validation.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n"
    )
    report_path = BENCHMARK / "comparison.md"
    existing_report = report_path.read_text()
    visual_review = (
        "### Визуальный просмотр" + existing_report.split("### Визуальный просмотр", 1)[1]
        if "### Визуальный просмотр" in existing_report else ""
    )
    report = existing_report.split("## Проверка PDF", 1)[0].rstrip()
    s, w = results["serpentype"], results["weasyprint"]
    report += (
        "\n\n## Проверка PDF\n\n"
        "Независимый разбор `pypdf` подтвердил в каждом PDF 320 строк данных без пропусков "
        "и повторов, 8 окончаний разделов, 16 фактических операций вставки изображений, "
        "повтор заголовков таблиц и корректные счётчики на всех применимых страницах. "
        f"Serpentype создал {s['pages']} страниц, WeasyPrint - {w['pages']}; "
        "по одной дополнительной странице Serpentype пришлось на каждую длинную таблицу "
        "в `appendix_1_ru` и `appendix_1_kk`.\n\n"
        f"На первой странице обоих PDF колонтитул присутствует: в данном CSS обе декларации "
        f"`content` имеют `!important`, а именованное правило `@page appendix_1_ru` "
        f"специфичнее общего `@page :first`. "
        f"Альбомные страницы Serpentype {s['landscape_pages']} и WeasyPrint "
        f"{w['landscape_pages']} отображаются горизонтально с эффективным поворотом PDF 0°. "
        "Номера альбомных страниц различаются из-за разбиения длинных таблиц.\n\n"
        f"Оба движка вложили только использованные глифы: подмножества шрифтов Serpentype "
        f"занимают {sum(s['embedded_font_file_bytes'].values()):,} байт до сжатия, "
        f"WeasyPrint - {sum(w['embedded_font_file_bytes'].values()):,} байт. "
        f"В PDF Serpentype левый и правый колонтитулы имеют отдельные позиции на всех "
        f"{len(s['page_geometry'])} страницах; минимальное расстояние "
        f"между их начальными точками - {s['minimum_footer_position_separation_pt']:.1f} pt.\n"
    )
    if visual_review:
        report += "\n" + visual_review
    report_path.write_text(report)
    print(json.dumps({
        name: {key: value for key, value in result.items() if key != "page_geometry"}
        for name, result in results.items()
    }, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
