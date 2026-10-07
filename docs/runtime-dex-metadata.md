# Explicit runtime DEX declarations

`java_external_metadata::DexMetadata` reads genuine ClassData declarations from
raw DEX or manifest-free DEX JAR/ZIP inputs. Method-pool references alone do not
create declarations. Exact flags, descriptors, complete Throws annotations,
hierarchy and attested member-class relationships are retained. Missing,
contradictory or duplicate metadata rejects instead of becoming an empty contract.

```rust,ignore
use decomp::java_external_metadata::{
    Classpath, DexMetadata, MetadataChain, MetadataProvider,
};

let runtime = DexMetadata::from_path("framework.jar")?;
let compiler = Classpath::from_paths(explicit_classpath)?;
let metadata = MetadataChain::new(vec![
    MetadataProvider::Dex(&runtime),
    MetadataProvider::Jvm(&compiler),
]);
```

Precedence selects an entire class owner. A missing method in the first owner's
declaration does not fall back to a different version of that class. Input APK
declarations still take precedence in the exporter. Providers are immutable;
changed inputs require a new provider and export.

Runtime DEX does not supply JVM Code or ConstantValue attributes. Metadata-only
methods with no JVM Code are not automatically native or abstract. A genuine
JVM Object bytecode proof remains accessible only from the selected JVM provider.
DEX encoded values are not treated as Java compile-time constants.

Both the `decomp` CLI and development profiler accept repeatable
`--metadata-dex PATH`; the CLI also honors repeatable `--classpath PATH` instead
of silently ignoring it. For example:

```sh
decomp app.apk out/ --workers 30 \
  --metadata-dex framework.jar \
  --classpath /opt/android-sdk/platforms/android-35/android.jar
```

Runtime providers precede the explicit JVM classpath, while first-owner
precedence and APK declaration priority remain intact. With no metadata flags,
the CLI uses the original `emit_java` path. Runtime DEX declarations do not
become a `javac` classpath; supply a real `.jar`/`.jmod`/`android.jar` separately
when compiler declarations are needed. Metadata setup time is reported by the
development profiler separately.

Runtime declaration availability is distinct from `javac` dependency availability:
a DEX-only framework archive cannot be passed to javac as a classfile archive.
Providing it does not establish whole-source compilation or Android execution.

## Current genuine Samsung evidence

The authorized Flip5's world-readable framework archive contains seven DEX files
and43,973 unique owners. Device and host SHA-256 match:
`1d329cf676fc029766bf37780fad61d835881e18ce2da570a3e165ed9f186a0b`.

- `UserHandle.semGetCallingUserId()I`: public static final, flags0x19.
- `View.semSetHoverPopupType(I)V`: public instance method, flags0x1.
- Both have fully decoded annotation metadata and no declared checked exceptions.

The device is Android16/SDK36, fingerprint
`samsung/b5qksx/b5q:16/BP4A.251205.006/F731NKSS6GZI1:user/release-keys`.
Its build match to the original Samsung Pass APK is **not attested**. Experiments
using this provider must remain separate from the Android35 baseline.

Receipt and declaration evidence are in
`bench/samsung-framework-metadata-receipt.json` and
`bench/samsung-framework-metadata-declarations.json`.
