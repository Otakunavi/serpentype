"""Repeatable stress corpus and separate-process PDF benchmarks for both engines.

Run from outside the repository so the installed serpentype wheel is imported:
  cd /tmp
  /path/to/serpentype/.venv/bin/python -I /path/to/serpentype/benchmarks/page_definition_stress.py

The saved HTML and CSS are byte-for-byte identical inputs for both engines.
"""

import argparse
import csv
import gc
import hashlib
import importlib.metadata
import json
import os
import platform
import re
import resource
import statistics
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ProcessPoolExecutor
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE_CSS = Path(
    "/Users/mini/Documents/dsrc/cameral-control/app/bg_worker/pdf_forming/templates/page_definition.css"
)
OUTPUT = ROOT / "output" / "benchmark" / "page_definition_stress"
PDF_OUTPUT = ROOT / "output" / "pdf"
_WORKER_ENGINE = None

os.environ.setdefault(
    "XDG_CACHE_HOME", str(Path(tempfile.gettempdir()) / "serpentype-fontconfig-cache")
)
Path(os.environ["XDG_CACHE_HOME"]).mkdir(parents=True, exist_ok=True)


def make_assets(directory):
    from PIL import Image, ImageDraw

    directory.mkdir(parents=True, exist_ok=True)
    qr = Image.new("RGB", (256, 256), "white")
    draw = ImageDraw.Draw(qr)
    for y in range(32):
        for x in range(32):
            if ((x * 17 + y * 31 + x * y * 7) % 13) < 6:
                draw.rectangle((x * 8, y * 8, x * 8 + 7, y * 8 + 7), fill="#173759")
    qr.save(directory / "qr.png")

    banner = Image.new("RGB", (640, 180), "#eaf2f8")
    draw = ImageDraw.Draw(banner)
    for i in range(16):
        x = i * 42
        draw.rectangle((x, 180 - (i * 29 % 125), x + 22, 180), fill="#3b7196")
    draw.line((0, 110, 639, 30), fill="#d27542", width=7)
    banner.save(directory / "chart.jpg", quality=90)


def render_source_css(source):
    # The application normally renders these Jinja expressions before PDF layout.
    css = re.sub(
        r'{% if cameral.document_type == "ADVICE" %}(.*?){% else %}(.*?){% endif %}',
        lambda match: match.group(2),
        source,
    )
    css = css.replace("{{ cameral.notice.reg_number }}", "TEST-2026-09-28")
    css = css.replace("{{ cameral.notice.reg_date }}", "28.09.2026")
    if "{{" in css or "{%" in css:
        raise ValueError("source CSS has unresolved Jinja expressions")
    return css


EXTRA_CSS = """
body { font-size: 9pt; line-height: 11pt; color: #172632 }
h1 { font-size: 15pt; line-height: 18pt; margin: 0 0 6pt 0 }
p { margin: 0 0 5pt 0; font-size: 9pt; line-height: 11pt }
.row { margin: 4pt 0 8pt 0; padding: 3pt }
.row p { margin: 0; font-size: 8pt; line-height: 10pt }
.banner { width: 90mm }
table { width: 180mm }
th, td { border: 0.4pt solid #617487; padding: 2pt; font-size: 8pt; line-height: 10pt }
th { background: #dce8f2; font-weight: 700 }
.appendix_2_ru table, .appendix_2_kk table { width: 267mm }
"""


SECTIONS = [
    ("appendix_1_ru", "Приложение 1. Реестр нарушений", 72),
    ("esf_turnover_notice_appendix_ru", "Приложение к уведомлению. Обороты ЭСФ", 16),
    ("esf_turnover_advice_appendix_ru", "Приложение к извещению. Обороты ЭСФ", 16),
    ("appendix_2_ru", "Приложение 2. Детализация по операциям", 56),
    ("appendix_1_kk", "1-қосымша. Бұзушылықтар тізілімі", 72),
    ("esf_turnover_notice_appendix_kk", "Хабарламаға қосымша. Айналымдар", 16),
    ("esf_turnover_advice_appendix_kk", "Хабархатқа қосымша. Айналымдар", 16),
    ("appendix_2_kk", "2-қосымша. Операциялар тізімі", 56),
]


def section_html(name, title, count, index):
    rows = []
    for number in range(1, count + 1):
        description = (
            f"Контрольная строка {name}-{number:03d}. "
            + "Подтверждение оборота и суммы по документу. " * (1 + number % 3)
        )
        rows.append(
            "<tr>"
            f"<td>{number}</td><td>{description}</td>"
            f"<td>{number * (index + 7):,}</td>"
            "</tr>"
        )
    grid = "".join(
        f"<p>Показатель {i + 1}<br>{(index + 1) * (i + 3) * 100}</p>"
        for i in range(5)
    )
    return (
        f'<section class="{name}">'
        f"<h1>{title}</h1>"
        f"<p>Контрольный документ TEST-2026-09-28. Раздел {index + 1} из 8.</p>"
        f'<div class="row">{grid}</div>'
        '<div class="qr"><img src="assets/qr.png"></div>'
        '<img class="banner" src="assets/chart.jpg">'
        "<table><thead><tr><th>№</th><th>Описание операции</th><th>Сумма</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody></table>"
        f"<p>Конец раздела {name}, строк: {count}.</p>"
        "</section>"
    )


def create_fixture(css_path, out_dir):
    out_dir.mkdir(parents=True, exist_ok=True)
    make_assets(out_dir / "assets")
    source = css_path.read_text()
    css = render_source_css(source) + EXTRA_CSS
    html = '<!doctype html><html lang="ru"><body><article data-print="paged">'
    html += "".join(section_html(*entry, index) for index, entry in enumerate(SECTIONS))
    html += "</article></body></html>"
    (out_dir / "stress_template.html").write_text(html)
    (out_dir / "rendered_page_definition.css").write_text(css)
    (out_dir / "fixture_manifest.json").write_text(
        json.dumps(
            {
                "source_css": str(css_path),
                "sections": [
                    {"page_name": name, "title": title, "rows": count}
                    for name, title, count in SECTIONS
                ],
                "total_table_rows": sum(count for _, _, count in SECTIONS),
                "image_elements": len(SECTIONS) * 2,
                "template_bytes": len(html.encode()),
                "css_bytes": len(css.encode()),
                "source_css_sha256": hashlib.sha256(source.encode()).hexdigest(),
                "template_sha256": hashlib.sha256(html.encode()).hexdigest(),
                "rendered_css_sha256": hashlib.sha256(css.encode()).hexdigest(),
                "image_sha256": {
                    path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in sorted((out_dir / "assets").iterdir())
                },
            },
            ensure_ascii=False,
            indent=2,
        ) + "\n"
    )


def make_engine(backend, out_dir):
    from serpentype import FontRegistry, Renderer, bundled_font_path

    html = (out_dir / "stress_template.html").read_text()
    css = (out_dir / "rendered_page_definition.css").read_text()
    if backend == "serpentype":
        fonts = FontRegistry()
        fonts.register_file(bundled_font_path(), family="Times Newer Roman")
        fonts.register_file(
            bundled_font_path(700), family="Times Newer Roman", weight=700
        )
        engine = Renderer(fonts=fonts, base_dir=str(out_dir), strict=True)
        return (
            lambda: engine.layout(html, css),
            lambda doc: doc.to_pdf(),
            lambda doc: doc.page_count,
        )
    from weasyprint import CSS, HTML
    from weasyprint.text.fonts import FontConfiguration

    config = FontConfiguration()
    face = (
        '@font-face { font-family: "Times Newer Roman"; '
        f'src: url("{Path(bundled_font_path()).as_uri()}"); font-weight:400 }}'
        '@font-face { font-family: "Times Newer Roman"; '
        f'src: url("{Path(bundled_font_path(700)).as_uri()}"); font-weight:700 }}'
    )
    sheet = CSS(string=face + css, base_url=str(out_dir), font_config=config)
    return (
        lambda: HTML(string=html, base_url=str(out_dir)).render(
            stylesheets=[sheet], font_config=config
        ),
        lambda doc: doc.write_pdf(),
        lambda doc: len(doc.pages),
    )


def peak_rss_bytes():
    value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return value if sys.platform == "darwin" else value * 1024


def current_rss_bytes():
    try:
        value = subprocess.check_output(
            ["ps", "-o", "rss=", "-p", str(os.getpid())], text=True
        )
        return int(value.strip()) * 1024
    except (OSError, ValueError, subprocess.CalledProcessError):
        return None


def timed_job(layout, export, pages):
    start = time.perf_counter_ns()
    cpu_start = time.process_time_ns()
    document = layout()
    middle = time.perf_counter_ns()
    pdf = export(document)
    end = time.perf_counter_ns()
    cpu_end = time.process_time_ns()
    return {
        "layout_ms": (middle - start) / 1e6,
        "export_ms": (end - middle) / 1e6,
        "total_ms": (end - start) / 1e6,
        "cpu_ms": (cpu_end - cpu_start) / 1e6,
        "pages": pages(document),
        "pdf_bytes": len(pdf),
    }, pdf


def initialize_worker(backend, out_dir):
    global _WORKER_ENGINE
    _WORKER_ENGINE = make_engine(backend, out_dir)
    # Warm each process before the measured workload.
    timed_job(*_WORKER_ENGINE)


def worker_job(_):
    if _WORKER_ENGINE is None:
        raise RuntimeError("worker engine was not initialized")
    record, _ = timed_job(*_WORKER_ENGINE)
    record["pid"] = os.getpid()
    record["peak_rss_bytes"] = peak_rss_bytes()
    return record


def summarize(records):
    return {
        key: {
            "median": statistics.median(record[key] for record in records),
            "min": min(record[key] for record in records),
            "max": max(record[key] for record in records),
        }
        for key in ("layout_ms", "export_ms", "total_ms", "cpu_ms")
    }


def measure(backend, out_dir, pdf_dir, repeats, load_jobs, workers):
    rss_before_init = current_rss_bytes()
    initialized_at = time.perf_counter_ns()
    layout, export, pages = make_engine(backend, out_dir)
    initialization_ms = (time.perf_counter_ns() - initialized_at) / 1e6
    rss_after_init = current_rss_bytes()
    pdf_dir.mkdir(parents=True, exist_ok=True)
    samples = []
    for index in range(repeats + 1):
        gc.collect()
        record, pdf = timed_job(layout, export, pages)
        record["phase"] = "cold" if index == 0 else "warm"
        record["peak_rss_bytes"] = peak_rss_bytes()
        record["current_rss_bytes"] = current_rss_bytes()
        samples.append(record)
        if index == 0:
            (pdf_dir / f"page_definition_stress_{backend}.pdf").write_bytes(pdf)
    load = {}
    for worker_count in workers:
        gc.collect()
        with ProcessPoolExecutor(
            max_workers=worker_count,
            initializer=initialize_worker,
            initargs=(backend, out_dir),
        ) as executor:
            # Starting workers and warming their engines is excluded from throughput.
            list(executor.map(worker_job, range(worker_count)))
            start = time.perf_counter_ns()
            task_records = list(executor.map(worker_job, range(load_jobs)))
            elapsed = (time.perf_counter_ns() - start) / 1e9
        worker_peaks = {}
        for record in task_records:
            pid = record["pid"]
            worker_peaks[pid] = max(worker_peaks.get(pid, 0), record["peak_rss_bytes"])
        load[str(worker_count)] = {
            "jobs": load_jobs,
            "wall_seconds": elapsed,
            "jobs_per_second": load_jobs / elapsed,
            "median_job_ms": statistics.median(
                record["total_ms"] for record in task_records
            ),
            "max_job_ms": max(record["total_ms"] for record in task_records),
            "sum_job_cpu_seconds": sum(record["cpu_ms"] for record in task_records) / 1000,
            "peak_worker_rss_bytes": max(worker_peaks.values()),
            "sum_peak_worker_rss_bytes": sum(worker_peaks.values()),
            "worker_pids": sorted(worker_peaks),
            "page_counts": sorted(set(record["pages"] for record in task_records)),
            "pdf_byte_counts": sorted(set(record["pdf_bytes"] for record in task_records)),
        }
    return {
        "backend": backend,
        "initialization_ms": initialization_ms,
        "rss_before_init_bytes": rss_before_init,
        "rss_after_init_bytes": rss_after_init,
        "samples": samples,
        "warm_summary": summarize(samples[1:]),
        "load": load,
        "process_peak_rss_bytes": peak_rss_bytes(),
        "python": sys.version.split()[0],
        "weasyprint": (
            __import__("weasyprint").__version__ if backend == "weasyprint" else None
        ),
    }


def report(results, out_dir, repeats, load_jobs, workers):
    serpentype, weasyprint = (results[name] for name in ("serpentype", "weasyprint"))
    a, b = serpentype["warm_summary"], weasyprint["warm_summary"]
    ratio = b["total_ms"]["median"] / a["total_ms"]["median"]
    lines = [
        "# Стресс-тест `page_definition.css`",
        "",
        f"Дата: {datetime.now(timezone.utc).isoformat(timespec='seconds')}. "
        f"Платформа: {platform.platform()}, Python {sys.version.split()[0]}, "
        f"{os.cpu_count()} логических CPU. Serpentype {importlib.metadata.version('serpentype')}, "
        f"WeasyPrint {weasyprint['weasyprint']}.",
        "",
        "Повторить из `/tmp`: "
        "`/Users/mini/Documents/serpentype/.venv/bin/python -I "
        "/Users/mini/Documents/serpentype/benchmarks/page_definition_stress.py "
        f"--repeats {repeats} --load-jobs {load_jobs} --workers {' '.join(map(str, workers))}`.",
        "",
        "Один и тот же сохранённый HTML/CSS подан в оба движка. Jinja-переменные заменены "
        "фиксированными тестовыми значениями. Семейство `Times Newer Roman` в обоих движках "
        "связано с одним и тем же Noto Sans Regular/Bold, чтобы сравнивать движки, а не установленные шрифты.",
        "",
        "Шаблон: 8 именованных разделов, 320 строк данных в 8 таблицах, "
        "16 тегов `<img>` (PNG и JPEG), QR flex, пятиколоночные grid-блоки, "
        "верхние/нижние поля, счётчики страниц и альбомные приложения.",
        "",
        f"После создания движка выполнены 1 холодный и {repeats} тёплых прохода. "
        "Один проход = вёрстка + экспорт PDF. Показатели времени в миллисекундах; "
        "медианы рассчитаны по тёплым проходам. Время и память инициализации приведены отдельно.",
        "",
        "| Показатель | Serpentype | WeasyPrint |",
        "| --- | ---: | ---: |",
        f"| Инициализация, мс | {serpentype['initialization_ms']:.1f} | {weasyprint['initialization_ms']:.1f} |",
        f"| RSS до инициализации, МиБ | {serpentype['rss_before_init_bytes']/2**20:.1f} | {weasyprint['rss_before_init_bytes']/2**20:.1f} |",
        f"| RSS после инициализации, МиБ | {serpentype['rss_after_init_bytes']/2**20:.1f} | {weasyprint['rss_after_init_bytes']/2**20:.1f} |",
        f"| Первый проход, мс | {serpentype['samples'][0]['total_ms']:.1f} | {weasyprint['samples'][0]['total_ms']:.1f} |",
        f"| Вёрстка, медиана, мс | {a['layout_ms']['median']:.1f} | {b['layout_ms']['median']:.1f} |",
        f"| Экспорт, медиана, мс | {a['export_ms']['median']:.1f} | {b['export_ms']['median']:.1f} |",
        f"| Полный проход, медиана, мс | {a['total_ms']['median']:.1f} | {b['total_ms']['median']:.1f} |",
        f"| Размер PDF, байт | {serpentype['samples'][0]['pdf_bytes']:,} | {weasyprint['samples'][0]['pdf_bytes']:,} |",
        f"| Число страниц | {serpentype['samples'][0]['pages']} | {weasyprint['samples'][0]['pages']} |",
        f"| Пиковый RSS процесса, МиБ | {serpentype['process_peak_rss_bytes']/2**20:.1f} | {weasyprint['process_peak_rss_bytes']/2**20:.1f} |",
        "",
        f"В этом сценарии медианный полный проход Serpentype быстрее в {ratio:.2f} раза. "
        "Это измерение относится к данному шаблону и оборудованию; разное число страниц "
        "означает различия в раскладке, даже если весь текст присутствует в обоих PDF.",
        "",
        f"Нагрузка: {load_jobs} независимых документов для каждого уровня параллелизма "
        f"({', '.join(map(str, workers))} процессов). Один прогретый движок на процесс; "
        "создание процессов и их прогрев исключены из замера пропускной способности. "
        "Время создания PDF включено.",
        "",
        "| Процессы | Serpentype, док./с | WeasyPrint, док./с | Serpentype RSS, сумма пиков, МиБ | WeasyPrint RSS, сумма пиков, МиБ |",
        "| ---: | ---: | ---: | ---: | ---: |",
    ]
    for count in workers:
        s, w = (results[name]["load"][str(count)] for name in ("serpentype", "weasyprint"))
        lines.append(
            f"| {count} | {s['jobs_per_second']:.2f} | {w['jobs_per_second']:.2f} "
            f"| {s['sum_peak_worker_rss_bytes']/2**20:.1f} | {w['sum_peak_worker_rss_bytes']/2**20:.1f} |"
        )
    lines += [
        "",
        "## Интерпретация и пределы",
        "",
        "- Автоматическая проверка PDF сохраняется в `validation.json`; выводы по ней добавляются ниже. "
        "PNG всех страниц и контактные листы находятся в `qa/`.",
        "- WeasyPrint и Serpentype могут по-разному переносить строки и страницы; "
        "сравнение скорости не подтверждает одинаковую геометрию PDF.",
        "- RSS включает интерпретатор, зависимости, шрифты и кэш каждого отдельного процесса. "
        "`ru_maxrss` показывает максимум за весь запуск, а не добавочную память одного документа.",
        "- Первый пилот с общей конфигурацией WeasyPrint в нескольких потоках завершился SIGABRT. "
        "Окончательный нагрузочный этап использует отдельные процессы для обоих движков.",
        "- Сырые значения каждого прохода сохранены в `results.json`; суммарный RSS процессов "
        "складывает пики отдельных рабочих процессов и может превышать их одновременное потребление.",
        "",
    ]
    (out_dir / "comparison.md").write_text("\n".join(lines))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--backend", choices=("serpentype", "weasyprint"))
    parser.add_argument("--source-css", type=Path, default=SOURCE_CSS)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--pdf-output", type=Path, default=PDF_OUTPUT)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--load-jobs", type=int, default=8)
    parser.add_argument("--workers", type=int, nargs="+", default=[1, 2, 4])
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.pdf_output = args.pdf_output.resolve()
    if args.backend:
        result = measure(
            args.backend, args.output, args.pdf_output,
            args.repeats, args.load_jobs, args.workers,
        )
        print(json.dumps(result, ensure_ascii=False))
        return
    create_fixture(args.source_css, args.output)
    results = {}
    for backend in ("serpentype", "weasyprint"):
        command = [
            sys.executable, "-I", str(Path(__file__).resolve()),
            "--backend", backend, "--output", str(args.output),
            "--pdf-output", str(args.pdf_output),
            "--repeats", str(args.repeats), "--load-jobs", str(args.load_jobs),
            "--workers", *map(str, args.workers),
        ]
        completed = subprocess.run(command, capture_output=True, text=True)
        (args.output / f"{backend}_stderr.log").write_text(completed.stderr)
        if completed.returncode:
            (args.output / f"{backend}_stdout.log").write_text(completed.stdout)
            raise RuntimeError(f"{backend} benchmark failed with exit code {completed.returncode}; see stderr log")
        results[backend] = json.loads(completed.stdout)
    payload = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(),
        "cpu_count": os.cpu_count(),
        "parameters": {
            "repeats": args.repeats,
            "load_jobs": args.load_jobs,
            "workers": args.workers,
        },
        "engines": results,
    }
    (args.output / "results.json").write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n"
    )
    with (args.output / "runs.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(
            stream,
            fieldnames=[
                "backend", "iteration", "phase", "layout_ms", "export_ms",
                "total_ms", "cpu_ms", "pages", "pdf_bytes",
                "current_rss_bytes", "peak_rss_bytes",
            ],
        )
        writer.writeheader()
        for backend, result in results.items():
            for iteration, sample in enumerate(result["samples"]):
                writer.writerow({"backend": backend, "iteration": iteration, **sample})
    with (args.output / "load.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(
            stream,
            fieldnames=[
                "backend", "processes", "jobs", "wall_seconds", "jobs_per_second",
                "median_job_ms", "max_job_ms", "sum_job_cpu_seconds",
                "sum_peak_worker_rss_bytes", "peak_worker_rss_bytes",
            ],
        )
        writer.writeheader()
        for backend, result in results.items():
            for processes, sample in result["load"].items():
                writer.writerow({
                    "backend": backend,
                    "processes": processes,
                    **{key: sample[key] for key in writer.fieldnames if key in sample},
                })
    report(results, args.output, args.repeats, args.load_jobs, args.workers)
    print(args.output / "comparison.md")


if __name__ == "__main__":
    main()
