# Absher library identification: LibScout experiment

Ran the existing LibScout detector against the original APK, with public
profiles, then independently checked scope and ambiguous matches.

## Inputs and tooling

- APK: `/home/jtjisgod/absher/base.apk`, SHA-256
  `172575d755606326dd5fd1d8c21dff7d3f5390ee782a0584746195952ac936e4`.
- LibScout commit `8c0bd2c147e818841fc695eecb1246e38458435f`.
- Public profiles: <https://github.com/reddr/LibScout-Profiles>, 402 libraries,
  8,542 versions; README last update 2019-07-16. Exact clone commit is in the receipt.
- Android35 metadata, JDK17, Gradle7.6.4, Java8 source/target. Build-only adjustments
  to the isolated checkout; detector algorithms/dependencies unchanged.

## Observations

The run completed in **699.63 seconds**, max RSS **4,325,180KiB**.
The raw result contains **46 matching profiles / 11 library-artifact names**,
including four names with full signature-profile matches and seven with
partial-only matches. Similarity is the tool's signature metric, not confidence
probability or body equivalence.

| Larger-profile candidate | Best signature score | Corroborating original DEX namespace |
|---|---:|---|
| Guava | 0.7329 | `com.google.common`: 2,020 class owners with methods |
| Retrofit | 0.8198 | `retrofit2`: 121 class owners with methods |
| Play services maps | 0.8262 | `com.google.android.gms.maps`: 304 class owners with methods |
| Firebase measurement connector impl | 0.9286 | Firebase family: 1,262 class owners with methods |
| Play services auth API phone | 0.7143 | `com.google.android.gms.auth.api.phone`: 9 class owners with methods |

These combined observations support library-family presence. Exact artifact
version, code ownership of every class, and license remain separate questions.
The public version candidates include Retrofit 2.6.0 and Guava 27.0-android;
those are reference-profile versions, not established installed versions.

## Why raw "full match" is not sufficient

- Facebook's 24 fully matching version profiles each contain **one class** with
  an identical hash. Its class-signature hash appears in **24,049 app classes**,
  including first-party-looking `com.elm.android.individual` classes. This is not
  accepted as Facebook identification or version evidence.
- Palette-KTX and Firebase IID-interop are also single-class profiles, matching
  30 and 8 class signatures respectively. They need independent evidence.
- The AndroidX and legacy support interpolator reference profiles have the same
  fingerprint. Their two detections are not independent confirmations.
- All exported matched root packages are empty. The partial-match
  `isOriginalPackageName` flag defaults to true, rather than proving original
  package identity. These fields cannot drive automatic source exclusion.

## Scope validation

A standalone probe used the same WALA scope/hierarchy loader and the original
DEX definitions: **all nine DEX files / 91,061 unique classes were loaded;
zero classes missing**. LibScout filters resource/synthetic/empty-inner classes,
reporting 71,459 classes; additional methodless-class pruning yields 68,837
class fingerprints. The display-name-enabled probe produced the same application
root hash as the original run: `e8798c3744e4c2899e36fb2c86ed2897`.

## Integration conclusion

Use this detector as a **candidate generator**, followed by profile-size and
signature-discrimination checks, fresh artifact profiles, original method/body
evidence and provenance/namespace validation. Do not treat unmatched code as
first-party, and do not mark broad `com.google.internal` as external. Library
presence does not establish open-source status or a license.

This raw LibScout output is not used directly as an export classifier. The
subsequent [verified-reference workflow](LIBRARY_SEPARATION.md) adds configurable
`--library-min-score`, exact implementation checks and original-owner routing.
Full commands, source/profile commits, raw JSON and validation probes are at:

`/tmp/opencode/absher-library-detection/RUN.md`

Primary result receipts:

- `/tmp/opencode/absher-library-detection/summary.json`
- `/tmp/opencode/absher-library-detection/results/sa/gov/moi/base_2679.json`
- `/tmp/opencode/absher-library-detection/scope-probe.jsonl`
- `/tmp/opencode/absher-library-detection/profile-probe.jsonl`
- `/tmp/opencode/absher-library-detection/match-footprint.jsonl`
- `/tmp/opencode/absher-library-detection/namespace-evidence.json`
