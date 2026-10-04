#!/usr/bin/env python3
"""Freeze a release binary and collect exact Samsung Pass coverage/compiler gates."""
import argparse
import datetime
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path


def sha(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="New artifact directory")
    parser.add_argument("--baseline", type=Path, help="Existing previous export directory")
    parser.add_argument("--workers", type=int, default=30)
    args = parser.parse_args()
    if args.workers < 1:
        parser.error("workers must be positive")
    repo = Path(__file__).resolve().parent.parent
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    gates = []

    def run(name, command):
        print(name, flush=True)
        with (root / f"{name}.log").open("w") as output:
            completed = subprocess.run(command, cwd=repo, stdout=output, stderr=subprocess.STDOUT)
        gates.append({"gate": name, "exit_code": completed.returncode})
        (root / "progress.json").write_text(json.dumps(gates, indent=2) + "\n")
        if completed.returncode:
            raise subprocess.CalledProcessError(completed.returncode, command)

    run("restore-inputs", [sys.executable, str(repo / "bench/restore_samsung_validation.py"), "--receipt", str(root / "inputs.json")])
    run("release-build", ["cargo", "build", "--release", "-p", "decomp", "--example", "java_profile"])
    binary = root / "java_profile.frozen"
    shutil.copy2(repo / "target/release/examples/java_profile", binary)
    inputs = json.loads((root / "inputs.json").read_text())
    jdk = Path(inputs["jdk_home"])
    apk = repo / "tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    framework = Path(inputs["framework"])
    cp = [Path("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
          Path("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
          Path("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod"),
          Path("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod")]
    sources = sorted([*repo.glob("crates/**/*.rs"), *repo.glob("crates/**/Cargo.toml"), repo / "Cargo.toml", repo / "Cargo.lock"])
    source_hashes = {str(path.relative_to(repo)): sha(path) for path in sources}
    for path in sources:
        destination = root / "source-snapshot" / path.relative_to(repo)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, destination)
    hashes = {str(path): sha(path) for path in [binary, apk, framework, *cp]}
    if hashes[str(apk)] != "d49a8bbf7aac35ec4d2e842535fd85ac105c49291d9efe23e041ae8afee12c41":
        raise ValueError("APK differs from original Samsung Pass goal input")
    environment = {"frozen_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                   "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
                   "source_hashes": source_hashes, "input_hashes": hashes,
                   "baseline_available": args.baseline is not None, "goal_complete": False}
    (root / "environment.json").write_text(json.dumps(environment, indent=2) + "\n")
    output = root / "java25"
    command = ["/usr/bin/time", "-v", "-o", str(root / "export.time"), str(binary), str(apk), str(output),
               "--workers", str(args.workers), "--source-release", "25", "--metadata-dex", str(framework)]
    for path in cp:
        command.extend(["--classpath", str(path)])
    run("export", command)
    changed = [name for name, digest in source_hashes.items() if not (repo / name).exists() or sha(repo / name) != digest]
    if changed or any(sha(Path(name)) != digest for name, digest in hashes.items()):
        raise ValueError(f"Frozen experiment inputs changed: {changed}")
    for line in (root / "export.log").open():
        if line.startswith("report="):
            (output / "report-debug.txt").write_text(line.removeprefix("report="))
            break
    run("inventory", [sys.executable, str(repo / "bench/samsung_pass_inventory.py"), str(apk), str(output), str(root / "inventory.json")])
    coverage = [sys.executable, str(repo / "bench/samsung_pass_goal_coverage.py"), str(output / "method-coverage.jsonl"), "--target-percent", "100", "--output", str(root / "coverage.json")]
    validation = [sys.executable, str(repo / "bench/samsung_pass_continue.py")]
    if args.baseline:
        coverage.extend(["--baseline", str(args.baseline / "method-coverage.jsonl"), "--require-no-regressions"])
        validation.append(str(args.baseline))
    run("coverage", coverage)
    validation.extend([str(output), str(root / "validation25"), "--jdk-home", str(jdk), "--source-release", "25"])
    run("compiler-validation", validation)
    # Compiler diagnostics and incomplete coverage may coexist with an exit-0
    # evidence collector. They never establish the user's full recovery goal.
    (root / "terminal.json").write_text(json.dumps({"gates": gates, "collector_finished": True,
        "baseline_available": args.baseline is not None, "goal_complete": False}, indent=2) + "\n")
    print("evidence=" + str(root), flush=True)


if __name__ == "__main__":
    main()
