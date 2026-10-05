#!/usr/bin/env python3
"""Alternate frozen checkpoint binaries with fresh outputs and retained OS caches."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import statistics
import subprocess
import time


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--pairs", type=int, default=2)
    args = parser.parse_args()
    if args.pairs < 1:
        parser.error("pairs must be positive")
    repo = Path(__file__).resolve().parent.parent
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    commands = {}
    for label, artifact in [("before", args.before.resolve()), ("after", args.after.resolve())]:
        first = (artifact / "export.time").read_text().splitlines()[0]
        command = shlex.split(first.split('Command being timed: "', 1)[1][:-1])
        expected = json.loads((artifact / "environment.json").read_text())["input_hashes"]
        for name, sha in expected.items():
            if digest(Path(name)) != sha:
                raise ValueError(f"Frozen input changed: {name}")
        commands[label] = command
    runs = []
    for pair in range(args.pairs):
        for label in (["before", "after"] if pair % 2 == 0 else ["after", "before"]):
            run = root / f"{pair}-{label}"
            run.mkdir()
            command = commands[label].copy()
            command[2] = str(run / "java25")
            started = time.monotonic()
            print(f"export {pair} {label}", flush=True)
            with (run / "export.log").open("w") as log:
                completed = subprocess.run(
                    ["/usr/bin/time", "-v", "-o", str(run / "export.time"), *command],
                    cwd=repo, stdout=log, stderr=subprocess.STDOUT,
                )
            if completed.returncode:
                raise subprocess.CalledProcessError(completed.returncode, command)
            item = {"pair": pair, "label": label, "artifact": str(run),
                    "wall_seconds": time.monotonic() - started}
            for line in (run / "java25/summary.txt").open():
                if line.startswith("profile_wall_s="):
                    item["profile"] = dict(re.findall(r"(\w+)=([^ ]+)", line.strip()))
                fields = line.replace("\\t", "\t").strip().split("\t")
                if fields[0] in ("serial", "throws_phase", "task_stage"):
                    item.setdefault(fields[0], {})[fields[1]] = float(fields[2])
            runs.append(item)
            (root / "progress.json").write_text(json.dumps(runs, indent=2) + "\n")
            print(f"finished {label}: {item['wall_seconds']:.3f}s", flush=True)
    medians = {label: statistics.median(r["wall_seconds"] for r in runs if r["label"] == label)
               for label in commands}
    result = {"runs": runs, "median_wall_seconds": medians,
              "median_reduction_percent": 100 * (1 - medians["after"] / medians["before"]),
              "limitations": "Same host, fresh process/output per run; OS cache retained. Repeated exports do not rerun javac. Other host workloads not isolated."}
    (root / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(medians), flush=True)


if __name__ == "__main__":
    main()
