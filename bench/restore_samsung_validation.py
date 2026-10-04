#!/usr/bin/env python3
"""Restore hash-pinned validation inputs; do not infer export completion."""
import argparse
import hashlib
import json
import subprocess
import tarfile
import urllib.request
from pathlib import Path


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def checked(path, expected):
    actual = digest(path)
    if actual != expected:
        raise ValueError(f"Checksum mismatch for {path}: {actual}")
    return actual


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--restore", action="store_true", help="Download JDK and pull framework if absent")
    parser.add_argument("--root", type=Path, default=Path("/tmp/opencode"))
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    bench = Path(__file__).resolve().parent
    jdk = json.loads((bench / "samsung-pass-java25-environment.json").read_text())
    platform = json.loads((bench / "samsung-framework-metadata-receipt.json").read_text())
    framework = args.root / "samsung-framework-metadata/framework.jar"
    fingerprint = None
    if not framework.exists() and args.restore:
        fingerprint = subprocess.check_output(
            ["adb", "-s", platform["serial"], "shell", "getprop", "ro.build.fingerprint"], text=True
        ).strip()
        if fingerprint != platform["fingerprint"]:
            raise ValueError("Device fingerprint differs from the recorded validation input")
        framework.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["adb", "-s", platform["serial"], "pull", platform["remote_path"], str(framework)], check=True)
    framework_hash = checked(framework, platform["sha256"])
    archive = args.root / "decomphify-jdk25/jdk.tar.gz"
    home = archive.parent / Path(jdk["home"]).name
    if not archive.exists() and args.restore:
        archive.parent.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(jdk["download"]["link"], archive)
    archive_hash = checked(archive, jdk["sha256_verified"])
    if not (home / "bin/java").exists() and args.restore:
        with tarfile.open(archive) as package:
            package.extractall(archive.parent, filter="data")
    version = subprocess.check_output([str(home / "bin/java"), "-version"], stderr=subprocess.STDOUT, text=True)
    if 'version "25.0.4.1"' not in version:
        raise ValueError("Unexpected Java runtime version")
    receipt = {
        "framework": str(framework), "framework_sha256": framework_hash,
        "recorded_device_fingerprint": platform["fingerprint"],
        "device_fingerprint_observed_this_run": fingerprint,
        "jdk_home": str(home), "jdk_archive_sha256": archive_hash,
        "java_version": version, "export_complete": False,
        "bounds": "Input restoration only; no source, compilation, or runtime-equivalence gate is implied",
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
