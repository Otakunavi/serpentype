# Historical benchmark: Serpentype 0.1 vs WeasyPrint 70.0

## 0.2 text and box-model gate

Measured on 2026-10-01 on macOS arm64 with CPython 3.13.3. The baseline is commit `de2a270` with its RustyBuzz path enabled; the candidate makes that same shaping path unconditional and adds the CSS box features. Both measurements use release ABI3 wheels. Five warm runs and five independent cold processes were compared with the same `simple`, forced-break, 60-row table and SVG corpus.

| Document | Warm layout change | Warm export change | Cold layout change | Cold export change |
| --- | ---: | ---: | ---: | ---: |
| Simple | +2.8% | +4.2% | -0.1% | +1.8% |
| Forced break | +4.7% | +1.9% | +4.2% | +0.5% |
| Table | +4.1% | -1.1% | +5.2% | +1.6% |
| SVG | +4.0% | 0.0% | +0.3% | +0.7% |

Peak RSS changed from 63.1 MiB to 67.4 MiB (+6.9%). All measured changes remain below the release limits of +15% layout, +10% export and +20% peak RSS. The current run retained the same page counts and uses the installed wheel. Raw measurements are retained in [`benchmark-results.jsonl`](../benchmark-results.jsonl) under the `de2a270-production-shaping` and `text-css-box-candidate` labels.

## Earlier comparison

Run on 2026-09-28 on macOS 15.6 arm64, CPython 3.13.3. Command:

```sh
python -I benchmarks.py --backend both --repeats 5 > benchmark-results.jsonl
```

`benchmarks.py` uses one long-lived process per engine, a common document corpus, the same Noto Sans Regular/Bold files, the same page CSS, and a writable Fontconfig cache for WeasyPrint. Serpentype registers fonts explicitly; WeasyPrint uses `@font-face`. The first document execution after engine setup is **cold**; five further executions are summarized by the median. Engine construction and font registration are outside the timing window. `layout` ends with a prepared document, `export` creates PDF bytes from it, and `total` includes both. Thread throughput uses the same engine and independent documents; peak RSS is measured in separate backend processes with `resource.getrusage`. The JSONL file keeps the full measurements.

| Document | Pages Serpentype / WeasyPrint | Warm layout ms | Warm export ms | Warm total ms | Cold total ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Simple, Serpentype | 1 / 1 | 0.26 | 4.80 | 5.06 | 5.47 |
| Simple, WeasyPrint | 1 / 1 | 10.79 | 2.60 | 13.19 | 38.35 |
| Forced break, Serpentype | 3 / 3 | 0.71 | 4.61 | 5.49 | 5.88 |
| Forced break, WeasyPrint | 3 / 3 | 17.58 | 5.27 | 22.71 | 32.01 |
| Table, Serpentype | 3 / 3 | 2.51 | 4.84 | 7.17 | 6.99 |
| Table, WeasyPrint | 3 / 3 | 82.39 | 17.60 | 100.33 | 119.84 |

Warm full-cycle ratios were 2.6×, 4.1× and 14.0× respectively on these specific templates. These measurements predate the font-subsetting change and should not be used to judge current PDF size or export time. Current results are in the stress-test report below. These measurements are not a general performance guarantee.

| Workers | Serpentype table jobs/s | WeasyPrint table jobs/s |
| ---: | ---: | ---: |
| 1 | 134.5 | 10.0 |
| 2 | 268.8 | 8.8 |
| 4 | 379.3 | 5.9 |
| 8 | 419.0 | 4.1 |

Peak RSS was **62.2 MiB** for the Serpentype benchmark process and **170.0 MiB** for the WeasyPrint process. The document table has 60 data rows. Poppler confirmed 3 pages from both and extracted all rows, repeated headers, and all 60 ruble signs. Visual inspection of the first and last pages found no clipped content. The engines distribute rows differently across the three pages and draw table borders differently, so identical page count does not mean pixel-identical output. The benchmark compares the supported controlled appearance and content, not browser fidelity.

## `page_definition.css` stress test

The separate [stress-test report](../output/benchmark/page_definition_stress/comparison.md) uses the source project's complete print stylesheet, eight named sections, 320 table rows, and 16 image placements. The [benchmark script](../benchmarks/page_definition_stress.py) saves the exact generated HTML/CSS, per-run timings, process load results, sample PDFs, and validation output. Its concurrent workload uses independent processes for both engines. The numbers above are from the earlier small corpus and should not be mixed with this larger scenario.
