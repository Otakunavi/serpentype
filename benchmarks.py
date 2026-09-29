"""Reproducible, same-process Serpentype/WeasyPrint timing on controlled documents.

Run from an installed wheel: python benchmarks.py --backend both --repeats 3
Only the font loading declaration differs: Serpentype imports bytes explicitly,
while WeasyPrint uses @font-face with the exact same TTF files.
"""
import argparse
import json
import os
import resource
import statistics
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# Give WeasyPrint/Fontconfig a writable cache even in a restricted workspace.
os.environ.setdefault("XDG_CACHE_HOME", str(Path(tempfile.gettempdir()) / "serpentype-fontconfig-cache"))
Path(os.environ["XDG_CACHE_HOME"]).mkdir(parents=True, exist_ok=True)


def corpus():
    css = """@page { size: A4; margin: 18mm }
      body, p, h1, table, th, td { font-family: 'Noto Sans'; color: #111111 }
      p { margin: 0 0 8pt 0; font-size: 11pt; line-height: 14pt }
      h1 { margin: 0 0 12pt 0; font-size: 20pt; line-height: 24pt }
      table { width: 170mm } th, td { border: 0.5pt solid #666666; padding: 3pt; font-size: 10pt; line-height: 12pt }
      th { background: #eeeeee } .breaker { break-after: page }"""
    simple = "<h1>Отчёт</h1>" + "".join(f"<p>Раздел {i}: краткое описание и сумма.</p>" for i in range(20))
    broken = "<h1>Договор</h1><p>Стороны согласовали условия.</p><div class='breaker'></div>" + "".join("<p>Подробное условие договора и текст на русском языке.</p>" for _ in range(50))
    rows = "".join(f"<tr><td style='width:10mm'>{i}</td><td style='width:130mm'>Описание позиции {i} с переменной длиной текста "+("длинное описание "*(i%5))+f"</td><td style='width:30mm'>{i*100} ₽</td></tr>" for i in range(60))
    table = "<h1>Реестр</h1><table><thead><tr><th style='width:10mm'>№</th><th style='width:130mm'>Описание</th><th style='width:30mm'>Сумма</th></tr></thead><tbody>"+rows+"</tbody></table>"
    return css, {"simple":simple,"broken":broken,"table":table}


def make_engine(backend, css):
    if backend == "serpentype":
        from serpentype import FontRegistry, Renderer, bundled_font_path
        fonts=FontRegistry()
        fonts.register_file(bundled_font_path(),family="Noto Sans")
        fonts.register_file(bundled_font_path(700),family="Noto Sans",weight=700)
        renderer=Renderer(fonts=fonts)
        def layout(html):return renderer.layout(html,css)
        def export(doc):return doc.to_pdf()
        def pages(doc):return doc.page_count
        return layout,export,pages
    from serpentype import bundled_font_path
    from weasyprint import CSS, HTML
    from weasyprint.text.fonts import FontConfiguration
    config=FontConfiguration()
    regular=Path(bundled_font_path()).as_uri()
    bold=Path(bundled_font_path(700)).as_uri()
    face=f"@font-face {{font-family:'Noto Sans';src:url('{regular}');font-weight:400}} @font-face {{font-family:'Noto Sans';src:url('{bold}');font-weight:700}}"
    sheet=CSS(string=face+css,font_config=config)
    def layout(html):return HTML(string=html).render(stylesheets=[sheet],font_config=config)
    def export(doc):return doc.write_pdf()
    def pages(doc):return len(doc.pages)
    return layout,export,pages


def measure(backend,repeats):
    css,documents=corpus()
    layout,export,pages=make_engine(backend,css)
    result={"backend":backend,"repeats":repeats,"documents":{},"throughput":{}}
    for name,html in documents.items():
        records=[]
        for _ in range(repeats+1):
            t0=time.perf_counter();doc=layout(html);t1=time.perf_counter();pdf=export(doc);t2=time.perf_counter()
            records.append({"layout_ms":(t1-t0)*1000,"export_ms":(t2-t1)*1000,"total_ms":(t2-t0)*1000,"pages":pages(doc),"pdf_bytes":len(pdf)})
        result["documents"][name]={"cold":records[0],"warm_median":{k:statistics.median(r[k] for r in records[1:]) for k in ("layout_ms","export_ms","total_ms")},"pages":records[-1]["pages"]}
    # Threads use the same long-lived engine and independent document instances.
    table=documents["table"]
    for workers in (1,2,4,8):
        jobs=max(8,workers*2)
        def task(_):return len(export(layout(table)))
        t0=time.perf_counter()
        with ThreadPoolExecutor(max_workers=workers) as pool:list(pool.map(task,range(jobs)))
        elapsed=time.perf_counter()-t0
        result["throughput"][str(workers)]={"jobs":jobs,"seconds":elapsed,"jobs_per_second":jobs/elapsed}
    result["peak_rss_bytes"]=resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return result


if __name__=="__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("--backend",choices=("serpentype","weasyprint","both"),default="both")
    parser.add_argument("--repeats",type=int,default=3)
    args=parser.parse_args()
    if args.backend=="both":
        # Separate processes make peak RSS and cold initialization comparable.
        for backend in ("serpentype","weasyprint"):
            output=subprocess.check_output([sys.executable,"-I",__file__,"--backend",backend,"--repeats",str(args.repeats)])
            print(output.decode().strip())
    else:
        print(json.dumps(measure(args.backend,args.repeats),ensure_ascii=False))
