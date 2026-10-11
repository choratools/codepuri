# Filesystem analysis recovery

`filesystem_recovery.py` exports original APK entries, readable resource trees,
ELF details/native assembly, Java source snapshots and every original DEX method
definition into one offline filesystem. It binds the method database to the APK
SHA-256 and preserves disambiguating DEX/pool/method identities and code-unit
offsets. Archive path escapes and duplicate entries reject.

```bash
python3 bench/filesystem_recovery.py \
  --apk /path/base.apk --database /path/analysis.sqlite \
  --java /path/full-java --java /path/current-app-java \
  --split /path/split_config.arm64_v8a.apk \
  --aapt2 /path/aapt2 --out /path/new-bundle
```

The database is produced by `analysis_bundle.py`; its adjacent `summary.json`
contains the original APK binding. Additional Java roots are independent source
snapshots, highest-numbered first for current recovery. Historical database Java
statuses are labelled historical, not silently promoted to current evidence.

## Files

- `java/N/`: source snapshots, with original identities/publication flags.
- `original/`: byte-preserved base APK entries; split entries under `splits/`.
- `resources/`: binary XML trees, original plain-text XML and resource tables.
- `components.json`: declared components/permissions/intent filter evidence.
- `methods.jsonl`: identity-to-file index for every original definition.
- `methods/XX/HASH.jsonl`: owner-grouped declaration and symbolic-call evidence.
- `methods/XX/HASH.dex.txt`: original disassembly or raw-code/no-body evidence.
- `native/`: ELF metadata and native executable disassembly.
- `receipt.json`: hashes, decoder commands/status, provenance and explicit gaps.

`--extend-splits` adds explicit splits to an existing bundle. Split DEX requires
its own inventory rather than being omitted. `--refresh-decoding` retries failed
native decoders, preserves plain XML and records resource-table absence. Original
failed command records remain as superseded history.

Native decoding uses architecture-specific GNU objdump. The ARM64 fallback uses
pyelftools + Capstone executable sections and preserves undecoded words. Opaque
non-ELF `.so` containers retain raw bytes/header evidence and are explicitly not
reported as recovered implementations. An offline package cannot recover server
authorization or prove unresolved runtime reflection targets.

## Absher observed bundle

Path: `/tmp/opencode/absher-filesystem-analysis/`.

- Base APK plus all 11 supplied ABI/density splits.
- 4,704 original entries SHA-256 verified after export.
- 475,340 original method definitions; 458,065 decoded bodies, zero DEX errors.
- Every indexed assembly offset checked against the original method identity.
- 227 declared application/components and 3,276 XML decoder observations.
- 92,475 Java snapshot files, including current Main/Login recovery.
- 50 native-library entries: 46 ELF images with assembly evidence and four
  opaque non-ELF face/OCR containers retained with explicit recovery gaps.
- No active decoder failures after retry/classification; absent ABI resource
  tables are recorded separately from decoder failures.

This is original-evidence recovery, not a claim that all missing Java/native
source, dynamic call targets, dependency closures or server checks are solved.
