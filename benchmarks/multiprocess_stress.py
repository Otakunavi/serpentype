"""Measure independent-process render throughput and bounded resource-cache use.

Run with an installed wheel: python benchmarks/multiprocess_stress.py --jobs 64
"""
from __future__ import annotations

import argparse
import json
import multiprocessing as mp
import os
import statistics
import time


def worker(worker_id: int, jobs: int) -> dict:
    from serpentype import FontRegistry, Renderer, ResourceKind, ResourceLoader, bundled_font_path

    fonts = FontRegistry()
    fonts.register_file(bundled_font_path(), "Noto Sans")
    renderer = Renderer(fonts=fonts)
    loader = ResourceLoader(
        mapping={f"memory:{i}": bytes([i % 251]) * 128_000 for i in range(32)},
        max_total_bytes=16_000_000,
        max_resources=64,
        cache_bytes=512_000,
    )
    elapsed_load = 0.0
    for i in range(32):
        start = time.perf_counter()
        loader.load(f"memory:{i}", ResourceKind.OTHER)
        elapsed_load += time.perf_counter() - start
        assert loader.cache_size <= loader.cache_bytes
    start = time.perf_counter()
    for i in range(jobs):
        document = renderer.layout(f"<p>Worker {worker_id}: item {i}</p>")
        assert document.to_pdf().startswith(b"%PDF")
    elapsed_render = time.perf_counter() - start
    return {
        "worker": worker_id,
        "jobs": jobs,
        "render_seconds": elapsed_render,
        "cache_load_seconds": elapsed_load,
        "cache_peak_bytes": loader.cache_size,
        "cache_limit_bytes": loader.cache_bytes,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workers", type=int, nargs="+", default=[1, 2, 4, 8])
    parser.add_argument("--jobs", type=int, default=64)
    args = parser.parse_args()
    if args.jobs < 1 or any(workers < 1 for workers in args.workers):
        parser.error("worker and job counts must be positive")
    report = {"pid": os.getpid(), "runs": []}
    for count in args.workers:
        per_worker = max(1, args.jobs // count)
        start = time.perf_counter()
        with mp.get_context("spawn").Pool(count) as pool:
            rows = pool.starmap(worker, [(i, per_worker) for i in range(count)])
        elapsed = time.perf_counter() - start
        completed = sum(row["jobs"] for row in rows)
        report["runs"].append({
            "workers": count,
            "jobs": completed,
            "wall_seconds": elapsed,
            "jobs_per_second": completed / elapsed,
            "cache_peak_bytes": max(row["cache_peak_bytes"] for row in rows),
            "worker_seconds_median": statistics.median(
                row["render_seconds"] for row in rows
            ),
        })
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
