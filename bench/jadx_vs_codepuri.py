#!/usr/bin/env python3
"""Paired cold-export benchmark: jadx vs codepuri's ordinary export path.

Protocol (docs/codepuri-plan.md): same APK, same worker count, fresh output
directories, /usr/bin/time -v, median across runs. jadx baseline on Samsung
Pass at 30 workers: 26.26 s wall / 16.1 GiB RSS (jadx 1.5.1, 2026-10-05).

Prerequisite: cargo build -p decomp --release --example java_export_bench

Example:
  bench/jadx_vs_codepuri.py tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk \
      --runs 4 --workers 30 --out bench/local-artifacts/paired-bench-<name>
"""
from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
JAVA_EXPORT = REPO_ROOT / "target/release/examples/java_export_bench"
JAVA_PROFILE = REPO_ROOT / "target/release/examples/java_profile"
DEFAULT_JADX = "/home/jtjisgod/tools/jadx/bin/jadx"

MARKERS = (
    "Method not decompiled",
    "Code decompiled incorrectly",
)


KEYS = (
    "Elapsed (wall clock) time (h:mm:ss or m:ss)",
    "User time (seconds)",
    "System time (seconds)",
    "Maximum resident set size (kbytes)",
)


def parse_time_v(path: Path) -> dict:
    fields = {}
    for line in path.read_text(errors="replace").splitlines():
        stripped = line.strip()
        for key in KEYS:
            if stripped.startswith(key + ":"):
                fields[key] = stripped[len(key) + 1:].strip()
                break
    elapsed = fields.get("Elapsed (wall clock) time (h:mm:ss or m:ss)", "0:0")
    parts = elapsed.split(":")
    h = int(parts[0]) if len(parts) == 3 else 0
    m = int(parts[-2])
    s = float(parts[-1])
    return {
        "wall_s": h * 3600 + m * 60 + s,
        "user_s": float(fields.get("User time (seconds)", "0")),
        "sys_s": float(fields.get("System time (seconds)", "0")),
        "max_rss_kib": int(fields.get("Maximum resident set size (kbytes)", "0")),
    }


def dir_stats(path: Path) -> dict:
    files = 0
    total = 0
    for entry in path.rglob("*"):
        if entry.is_file():
            files += 1
            total += entry.stat().st_size
    return {"files": files, "bytes": total}


def count_markers(path: Path) -> dict:
    counts = {marker: 0 for marker in MARKERS}
    for entry in path.rglob("*.java"):
        try:
            text = entry.read_text(errors="ignore")
        except OSError:
            continue
        for marker in MARKERS:
            counts[marker] += text.count(marker)
    return counts


def timed_run(argv: list, time_out: Path, log_out: Path) -> dict:
    cmd = ["/usr/bin/time", "-v", "-o", str(time_out), *argv]
    start = time.monotonic()
    with log_out.open("w") as log:
        proc = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, check=False)
    elapsed = time.monotonic() - start
    result = parse_time_v(time_out)
    result["exit_code"] = proc.returncode
    result["monotonic_s"] = round(elapsed, 3)
    return result


def run_jadx(jadx_bin: Path, apk: Path, out: Path, workers: int, time_out: Path, log_out: Path) -> dict:
    run = timed_run([str(jadx_bin), "--no-res", "-j", str(workers), "-d", str(out), str(apk)],
                    time_out, log_out)
    run.update(dir_stats(out))
    return run


def run_codepuri(
    apk: Path,
    out: Path,
    workers: int,
    time_out: Path,
    log_out: Path,
    profiled: bool,
    fast_proofs: bool,
) -> dict:
    binary = JAVA_PROFILE if profiled else JAVA_EXPORT
    argv = [str(binary), str(apk), str(out), "--workers", str(workers)]
    if fast_proofs:
        argv.append("--fast-proofs")
    run = timed_run(argv, time_out, log_out)
    run.update(dir_stats(out))
    return run


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=4)
    parser.add_argument("--workers", type=int, default=30)
    parser.add_argument("--jadx-bin", type=Path, default=Path(DEFAULT_JADX))
    parser.add_argument("--profiled-codepuri", action="store_true",
                        help="use diagnostic java_profile instead of the minimal unprofiled exporter")
    parser.add_argument("--fast-proofs-codepuri", action="store_true",
                        help="enable opt-in fast_proofs mode (may alter unpublished reasons on other APKs)")
    parser.add_argument("--skip-jadx", action="store_true")
    parser.add_argument("--skip-codepuri", action="store_true")
    parser.add_argument("--markers", action="store_true", help="count jadx failure markers (slow)")
    args = parser.parse_args()

    apk = args.apk.resolve()
    out_root = args.out.resolve()
    if not apk.is_file():
        parser.error(f"APK not found: {apk}")
    if out_root.exists():
        parser.error(f"output directory must not exist: {out_root}")
    codepuri_binary = JAVA_PROFILE if args.profiled_codepuri else JAVA_EXPORT
    if not args.skip_codepuri and not codepuri_binary.is_file():
        parser.error(f"missing {codepuri_binary}; build the selected codepuri export example first")
    if not args.skip_jadx and not args.jadx_bin.is_file():
        parser.error(f"jadx binary not found: {args.jadx_bin}")

    out_root.mkdir(parents=True)
    report = {
        "apk": str(apk),
        "workers": args.workers,
        "runs": args.runs,
        "codepuri_profiled": args.profiled_codepuri,
        "codepuri_fast_proofs": args.fast_proofs_codepuri,
        "jadx_version": None,
        "samples": [],
    }
    if not args.skip_jadx:
        version = subprocess.run([str(args.jadx_bin), "--version"], capture_output=True, text=True)
        report["jadx_version"] = (version.stdout + version.stderr).strip().splitlines()[-1] if version.returncode == 0 else None

    for index in range(args.runs):
        sample_dir = out_root / f"run{index}"
        sample_dir.mkdir()
        order = ["jadx", "codepuri"] if index % 2 == 0 else ["codepuri", "jadx"]
        if args.skip_jadx:
            order.remove("jadx")
        if args.skip_codepuri:
            order.remove("codepuri")
        sample: dict = {"run": index, "order": order}
        jadx_out = sample_dir / "jadx-output"
        for tool in order:
            if tool == "jadx":
                sample["jadx"] = run_jadx(
                    args.jadx_bin,
                    apk,
                    jadx_out,
                    args.workers,
                    sample_dir / "jadx.time",
                    sample_dir / "jadx.log",
                )
            else:
                cp_out = sample_dir / "codepuri-output"
                sample["codepuri"] = run_codepuri(
                    apk,
                    cp_out,
                    args.workers,
                    sample_dir / "codepuri.time",
                    sample_dir / "codepuri.log",
                    args.profiled_codepuri,
                    args.fast_proofs_codepuri,
                )
        if args.markers and "jadx" in sample:
            sample["jadx"]["markers"] = count_markers(jadx_out)
        report["samples"].append(sample)
        print(json.dumps({k: v for k, v in sample.items()}, indent=1), flush=True)

    summary = {}
    for tool in ("jadx", "codepuri"):
        walls = [s[tool]["wall_s"] for s in report["samples"] if tool in s]
        rss = [s[tool]["max_rss_kib"] for s in report["samples"] if tool in s]
        if walls:
            summary[tool] = {
                "wall_median_s": round(statistics.median(walls), 2),
                "wall_min_s": round(min(walls), 2),
                "max_rss_median_gib": round(statistics.median(rss) / 1024 / 1024, 2),
            }
    report["summary"] = summary
    deltas = [
        sample["codepuri"]["wall_s"] - sample["jadx"]["wall_s"]
        for sample in report["samples"]
        if "codepuri" in sample and "jadx" in sample
    ]
    if deltas:
        report["summary"]["paired_wall_delta_median_s"] = round(
            statistics.median(deltas), 2
        )
    report_path = out_root / "paired-benchmark.json"
    report_path.write_text(json.dumps(report, indent=1) + "\n")

    print(f"\n== summary ({report['jadx_version'] or 'jadx'}) ==")
    for tool in ("jadx", "codepuri"):
        if tool not in summary:
            continue
        stats = summary[tool]
        print(f"{tool}: wall median {stats['wall_median_s']}s, "
              f"min {stats['wall_min_s']}s, RSS {stats['max_rss_median_gib']} GiB")
    if deltas:
        print(f"paired delta median (codepuri - jadx): {report['summary']['paired_wall_delta_median_s']}s")
    print(f"report: {report_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
