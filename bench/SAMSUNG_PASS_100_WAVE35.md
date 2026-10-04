# Samsung Pass wave35: immutable signature cache measurement

Same original APK and ordered classpath as wave34; frozen source changes are limited to checked_source_bridge.rs and its cache tests.

- Export wall time: 273.94 s; wave34: 276.59 s. This small single-run difference is not a robust speedup claim.
- Executable acceptance: 183,646 / 194,286 = 94.523537%; gained/lost identities: 0 / 0.
- Sources: 32,420; syntax gate exit 0; full uncapped javac: 50,711 diagnostics, exit 1.
- Both wave34 and wave26 original-identity no-loss gates pass.
- Cache's bounded exact-source tests pass, but full output byte equality is NOT established: 5,311 files differ, including dispatcher numbering. Attribution is being investigated independently.
- No whole-app compilation, Android runtime equivalence, or 100% completion claim.

Receipts: bench/samsung-pass-wave35-cache-final.json and /tmp/opencode/samsung-pass-100-wave35/.

Next verified candidates: mutable constructor stores incorrectly receive final-only loop restrictions; absent Throws metadata on final concrete InputStream implementations blocks inherited-contract derivation. Original JADX output informs these fixes but its omitted throws clauses are not a compilable oracle.
