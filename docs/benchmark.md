# Historical reference measurements: Serpentype 0.1 and WeasyPrint 70.0

WeasyPrint is included as a reference implementation for the controlled
fixtures. The historical timing and memory figures below are environment-
specific diagnostics and do not describe a general performance target.

## 0.2 bounded-cache and independent-process stress

Measured on 2026-10-05 on macOS arm64, CPython 3.13, against the locally installed optimized ABI3 wheel. `benchmarks/multiprocess_stress.py --jobs 64` ran independent worker processes, each rendering PDFs and loading 32 distinct 128 KB resources into a 512,000-byte LRU. Peak cache occupancy was exactly 512,000 bytes in every run and never exceeded its configured bound. Aggregate render throughput was 856.5, 1,085.7, 980.6 and 635.4 jobs/s for 1, 2, 4 and 8 processes, respectively. This short smoke measurement verifies the benchmark and bounded-cache gate; it is not a stable performance comparison. Raw data is in [`benchmark-results.jsonl`](../benchmark-results.jsonl) under `release-hardening-optimized-wheel`.

## 0.2 positioning, Flex and Grid gate

Measured on 2026-10-05 on macOS arm64 with the installed release ABI3 wheel. The immediately preceding `resources-svg-candidate` measurement is the baseline; the candidate is the median of five independent processes with seven warm repeats each. Warm layout changes for simple, forced-break, table and SVG documents were -11.0%, -7.9%, -4.3% and -5.9%; export changes were -19.8%, -20.7%, -12.7% and -17.0%. PDF sizes and page counts were unchanged. Median candidate peak RSS was 85.9 MB versus the retained 71.7 MB baseline (+19.7%), below the +20% release gate. All layout, export and RSS regression gates passed. Raw aggregate measurements are retained in [`benchmark-results.jsonl`](../benchmark-results.jsonl) under `position-flex-grid-candidate`.

## 0.2 resources and vector SVG gate

Measured on 2026-10-05 on macOS arm64 with release ABI3 wheels. The baseline is commit `7aa114e`; the candidate adds object fitting, raster downsampling, vector SVG and direct resource-loader integration. Warm median layout/export changes for simple, forced-break, table and SVG documents were respectively -18.9%/-13.8%, -5.0%/+0.5%, +0.7%/-1.1% and -6.0%/-44.1%. SVG PDF size fell from 14,265 to 8,450 bytes. Peak RSS changed from 72.0 MB to 71.7 MB (-0.4%). All layout, export and RSS regression gates passed. Raw measurements are retained in [`benchmark-results.jsonl`](../benchmark-results.jsonl) under `resources-svg-baseline-7aa114e` and `resources-svg-candidate`.

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
