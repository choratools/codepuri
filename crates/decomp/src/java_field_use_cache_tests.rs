#![allow(clippy::unwrap_used)]
use super::*;
use crate::java_aliases::{AliasInventory, AliasMode};
use dex_format::{
    RawSpan,
    class::{ClassData, EncodedField},
    ids::{FieldId, TypeId},
};
use std::sync::Barrier;

const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };

struct EmptyMetadata;
impl ExternalMetadata for EmptyMetadata {
    fn method(
        &self,
        _: &crate::java_aliases::MethodKey,
    ) -> Option<crate::java_external_metadata::MethodMetadata> {
        None
    }
}

fn string(dex: &mut DexFile, text: &str) -> u32 {
    let mut value = dex.strings[0].clone();
    value.value = text.into();
    let index = dex.strings.len() as u32;
    dex.strings.push(value);
    index
}

fn ty(dex: &mut DexFile, text: &str) -> u32 {
    let descriptor_idx = string(dex, text);
    let index = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}

fn fixture(owner: &str, parent: Option<&str>) -> DexFile {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let mut class = dex.classes[0].clone();
    class.class_idx = ty(&mut dex, owner);
    class.superclass_idx = parent.map_or(u32::MAX, |parent| ty(&mut dex, parent));
    class.access_flags = 1;
    class.interfaces = None;
    class.annotations = None;
    class.class_data_off = 1;
    class.static_values_off = 0;
    class.static_values = None;
    class.class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![],
        virtual_methods: vec![],
        span: SPAN,
    });
    dex.classes = vec![class];
    dex.field_ids.clear();
    dex
}

fn field(dex: &mut DexFile, name: &str, descriptor: &str, flags: u32) -> FieldKey {
    let type_idx = u16::try_from(ty(dex, descriptor)).unwrap();
    let name_idx = string(dex, name);
    let field_idx = dex.field_ids.len() as u32;
    dex.field_ids.push(FieldId {
        class_idx: u16::try_from(dex.classes[0].class_idx).unwrap(),
        type_idx,
        name_idx,
        span: SPAN,
    });
    let data = dex.classes[0].class_data.as_mut().unwrap();
    let fields = if flags & 8 == 0 {
        &mut data.instance_fields
    } else {
        &mut data.static_fields
    };
    fields.push(EncodedField {
        field_idx,
        access_flags: flags,
    });
    super::super::operand_key(dex, field_idx).unwrap()
}

fn aliases(inputs: &[DexFile]) -> JavaAliases {
    let mut inventory = AliasInventory::default();
    super::super::FieldPublication::collect(inputs, &mut inventory).unwrap();
    inventory.build(AliasMode::Ascii)
}

fn context<'a>(
    caller: &'a str,
    receiver: Option<&'a str>,
    is_static: bool,
) -> FieldAccessContext<'a> {
    FieldAccessContext {
        caller,
        receiver,
        is_static,
    }
}

#[test]
fn input_access_hidden_receivers_and_typed_failures_match_uncached_proofs() {
    let mut base = fixture("Lp/Base;", None);
    let public = field(&mut base, "value", "I", 1);
    let protected = field(&mut base, "protectedValue", "I", 4);
    let private = field(&mut base, "privateValue", "I", 2);
    let static_field = field(&mut base, "STATIC", "I", 9);
    let mut child = fixture("Lq/Child;", Some("Lp/Base;"));
    field(&mut child, "value", "Ljava/lang/String;", 1);
    let inputs = [base, child];
    let names = aliases(&inputs);
    let index = InputIndex::new(&inputs);
    let cache = FieldUseCache::new(true, &inputs, &EmptyMetadata, &names, Some(&index));
    let cases = [
        (&public, context("Lq/Child;", Some("Lq/Child;"), false)),
        (&protected, context("Lq/Child;", Some("Lq/Child;"), false)),
        (&protected, context("Lq/Child;", Some("Lp/Base;"), false)),
        (&private, context("Lq/Child;", Some("Lq/Child;"), false)),
        (&private, context("Lp/Base;", Some("Lq/Child;"), false)),
        (&public, context("Lq/Child;", None, false)),
        (&static_field, context("Lq/Child;", None, true)),
        (&static_field, context("Lq/Child;", None, false)),
    ];
    for (key, access) in &cases {
        let resolved = resolve_field_indexed(&inputs, key, &EmptyMetadata, access, Some(&index));
        let source = resolved
            .clone()
            .map_err(PublicationBlocker::Resolution)
            .and_then(|resolved| {
                source_use_indexed(
                    &inputs,
                    &resolved,
                    &EmptyMetadata,
                    access,
                    &names,
                    Some(&index),
                )
            });
        for _ in 0..2 {
            assert_eq!(cache.resolve(key, access), resolved);
            assert_eq!(cache.source_use(key, access), source);
            assert_eq!(
                cache.retain_receiver(key, access),
                source
                    .clone()
                    .map(|source| source.receiver == super::super::FieldReceiver::Retain)
            );
        }
    }
    assert_eq!(
        cache.source_use(&public, &cases[0].1).unwrap().receiver,
        super::super::FieldReceiver::CastToOwner
    );
    assert!(matches!(
        cache.resolve(&protected, &cases[2].1),
        Err(FieldRequirement::Access(_))
    ));
    assert!(matches!(
        cache.resolve(&static_field, &cases[7].1),
        Err(FieldRequirement::Staticness(_))
    ));
    let stats = cache.stats();
    assert_eq!(stats.entries, cases.len());
    assert_eq!(stats.resolution.misses, cases.len());
    assert_eq!(stats.source_use.misses, cases.len());
    assert_eq!(stats.retain.misses, cases.len());
    assert!(stats.resolution.failures > 0);
    assert!(stats.source_use.failures > stats.resolution.failures);
}

#[test]
fn exact_symbolic_keys_caller_receiver_and_opcode_are_isolated() {
    let mut base = fixture("Lp/Base;", None);
    let key = field(&mut base, "value", "I", 1);
    let child = fixture("Lp/Child;", Some("Lp/Base;"));
    let inputs = [base, child];
    let names = aliases(&inputs);
    let index = InputIndex::new(&inputs);
    let cache = FieldUseCache::new(true, &inputs, &EmptyMetadata, &names, Some(&index));
    let original = context("Lp/Base;", Some("Lp/Base;"), false);
    assert!(cache.resolve(&key, &original).is_ok());
    let inherited = FieldKey {
        owner: "Lp/Child;".into(),
        ..key.clone()
    };
    assert_eq!(
        cache.resolve(&inherited, &original),
        cache.resolve(&key, &original)
    );
    let wrong_type = FieldKey {
        descriptor: "J".into(),
        ..key.clone()
    };
    assert!(matches!(
        cache.resolve(&wrong_type, &original),
        Err(FieldRequirement::Declaration(_))
    ));
    let wrong_name = FieldKey {
        name: "other".into(),
        ..key.clone()
    };
    assert!(matches!(
        cache.resolve(&wrong_name, &original),
        Err(FieldRequirement::Declaration(_))
    ));
    for access in [
        context("Lp/Child;", Some("Lp/Base;"), false),
        context("Lp/Base;", Some("Lp/Child;"), false),
        context("Lp/Base;", None, false),
    ] {
        assert!(cache.resolve(&key, &access).is_ok());
    }
    assert!(matches!(
        cache.resolve(&key, &context("Lp/Base;", Some("Lp/Base;"), true)),
        Err(FieldRequirement::Staticness(_))
    ));
    assert_eq!(cache.stats().entries, 8);
    assert_eq!(cache.stats().resolution.misses, 8);
}

#[test]
fn canonical_operand_identity_is_shared_across_distinct_dex_pools() {
    let mut base = fixture("Lp/Base;", None);
    let key = field(&mut base, "value", "I", 1);
    let mut child = fixture("Lp/Child;", Some("Lp/Base;"));
    let class_idx = u16::try_from(ty(&mut child, "Lp/Base;")).unwrap();
    let type_idx = u16::try_from(ty(&mut child, "I")).unwrap();
    let name_idx = string(&mut child, "value");
    child.field_ids.push(FieldId {
        class_idx,
        type_idx,
        name_idx,
        span: SPAN,
    });
    let inputs = [base, child];
    let names = aliases(&inputs);
    let index = InputIndex::new(&inputs);
    let cache = FieldUseCache::new(true, &inputs, &EmptyMetadata, &names, Some(&index));
    let access = context("Lp/Child;", Some("Lp/Child;"), false);
    let first = super::super::operand_key(&inputs[0], 0).unwrap();
    let second = super::super::operand_key(&inputs[1], 0).unwrap();
    assert_eq!(first, key);
    assert_eq!(second, key);
    assert_eq!(
        cache.source_use(&first, &access),
        cache.source_use(&second, &access)
    );
    let stats = cache.stats();
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.resolution.misses, 1);
    assert_eq!(stats.source_use.misses, 1);
    assert_eq!(stats.source_use.hits, 1);
}

#[test]
fn concurrent_positive_and_negative_stages_compute_once() {
    let mut base = fixture("Lp/Base;", None);
    let key = field(&mut base, "value", "I", 1);
    let inputs = [base];
    let names = aliases(&inputs);
    let index = InputIndex::new(&inputs);
    for access in [
        context("Lp/Base;", Some("Lp/Base;"), false),
        context("Lp/Base;", Some("Lp/Base;"), true),
    ] {
        let cache = FieldUseCache::new(true, &inputs, &EmptyMetadata, &names, Some(&index));
        let barrier = Barrier::new(8);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        (
                            cache.resolve(&key, &access),
                            cache.source_use(&key, &access),
                            cache.retain_receiver(&key, &access),
                        )
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(results.iter().all(|result| result == &results[0]));
        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        for stage in [stats.resolution, stats.source_use, stats.retain] {
            assert_eq!(stage.misses, 1);
            assert_eq!(stage.failures, usize::from(access.is_static));
            assert!(stage.hits >= 7);
        }
    }
}

#[test]
fn aliases_and_input_snapshots_are_isolated_and_disabled_path_retains_nothing() {
    let mut base = fixture("Lp/Base;", None);
    let key = field(&mut base, "value", "I", 1);
    let inputs = [base];
    let names = aliases(&inputs);
    let missing_names = JavaAliases::default();
    let index = InputIndex::new(&inputs);
    let access = context("Lp/Base;", Some("Lp/Base;"), false);
    let first = FieldUseCache::new(true, &inputs, &EmptyMetadata, &names, Some(&index));
    let other = FieldUseCache::new(true, &inputs, &EmptyMetadata, &missing_names, Some(&index));
    assert!(first.source_use(&key, &access).is_ok());
    assert!(other.source_use(&key, &access).is_err());
    let changed = [fixture("Lp/Base;", None)];
    let changed_index = InputIndex::new(&changed);
    let changed_cache =
        FieldUseCache::new(true, &changed, &EmptyMetadata, &names, Some(&changed_index));
    assert!(matches!(
        changed_cache.resolve(&key, &access),
        Err(FieldRequirement::Declaration(_))
    ));
    let disabled = FieldUseCache::new(false, &inputs, &EmptyMetadata, &names, Some(&index));
    for _ in 0..3 {
        assert_eq!(
            disabled.resolve(&key, &access),
            first.resolve(&key, &access)
        );
        assert_eq!(
            disabled.source_use(&key, &access),
            first.source_use(&key, &access)
        );
        assert_eq!(
            disabled.retain_receiver(&key, &access),
            first.retain_receiver(&key, &access)
        );
    }
    assert_eq!(disabled.stats(), FieldUseStats::default());
    let mut copied = first.source_use(&key, &access).unwrap();
    copied.owner = "corrupted".into();
    assert_ne!(first.source_use(&key, &access).unwrap(), copied);
}

#[test]
fn panic_leaves_uninitialized_stage_retryable() {
    let cell = OnceLock::<Result<String, FieldRequirement>>::new();
    let counters = Counters::default();
    assert!(
        std::panic::catch_unwind(|| counters.complete(&cell, || panic!("failed initializer")))
            .is_err()
    );
    assert_eq!(counters.stats(), StageStats::default());
    assert_eq!(
        counters.complete(&cell, || Ok("proof".into())),
        Ok("proof".into())
    );
    assert_eq!(
        counters.complete(&cell, || panic!("cache hit")),
        Ok("proof".into())
    );
    assert_eq!(
        counters.stats(),
        StageStats {
            hits: 1,
            misses: 1,
            failures: 0
        }
    );
}

#[test]
fn genuine_classpath_proof_is_lazy_and_matches_disabled_results() {
    use crate::java_external_metadata::Classpath;
    use std::{path::PathBuf, process::Command};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temp = Temp(std::env::temp_dir().join(format!(
        "field-use-cache-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&temp.0).unwrap();
    let base = temp.0.join("Base.java");
    let child = temp.0.join("Child.java");
    std::fs::write(&base, "package p; public class Base { public int value; public int inherited; protected int guarded; private int secret; public static int STATIC; }").unwrap();
    std::fs::write(
        &child,
        "package q; public class Child extends p.Base { public String value; }",
    )
    .unwrap();
    let javac = std::env::var_os("JAVA_HOME")
        .map(|home| PathBuf::from(home).join("bin/javac"))
        .unwrap_or_else(|| {
            if Command::new("javac").arg("-version").output().is_ok() {
                PathBuf::from("javac")
            } else {
                PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac")
            }
        });
    let compiler_home = javac
        .parent()
        .and_then(std::path::Path::parent)
        .filter(|home| home.join("jmods/java.base.jmod").is_file())
        .map(std::path::Path::to_path_buf);
    let output = Command::new(javac)
        .arg("-d")
        .arg(&temp.0)
        .args([base, child])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut metadata =
        Classpath::from_paths([temp.0.join("p/Base.class"), temp.0.join("q/Child.class")]).unwrap();
    let java_home = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .filter(|home| home.join("jmods/java.base.jmod").is_file())
        .or(compiler_home)
        .or_else(|| {
            let settings = Command::new("java")
                .args(["-XshowSettings:properties", "-version"])
                .output()
                .ok()?;
            String::from_utf8_lossy(&settings.stderr)
                .lines()
                .find_map(|line| line.trim().strip_prefix("java.home = ").map(PathBuf::from))
                .filter(|home| home.join("jmods/java.base.jmod").is_file())
        })
        .unwrap_or_else(|| PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"));
    // A protected-receiver negative traverses Base's superclass to prove that
    // Base is not a subtype of Child. Supply the genuine Object declaration.
    metadata
        .add_path(java_home.join("jmods/java.base.jmod"))
        .unwrap();
    let key = |name: &str| FieldKey {
        owner: "Lp/Base;".into(),
        name: name.into(),
        descriptor: "I".into(),
    };
    let mut inventory = AliasInventory::default();
    inventory.add_type("Lq/Child;").unwrap();
    for name in ["value", "inherited", "guarded", "secret", "STATIC"] {
        inventory.add_field(key(name)).unwrap();
    }
    let names = inventory.build(AliasMode::Ascii);
    let cache = FieldUseCache::new(true, &[], &metadata, &names, None);
    let disabled = FieldUseCache::new(false, &[], &metadata, &names, None);
    let cases = [
        (key("value"), context("Lq/Child;", Some("Lq/Child;"), false)),
        (
            key("inherited"),
            context("Lq/Child;", Some("Lq/Child;"), false),
        ),
        (
            key("guarded"),
            context("Lq/Child;", Some("Lq/Child;"), false),
        ),
        (
            key("guarded"),
            context("Lq/Child;", Some("Lp/Base;"), false),
        ),
        (
            key("secret"),
            context("Lq/Child;", Some("Lq/Child;"), false),
        ),
        (key("STATIC"), context("Lq/Child;", None, true)),
        (key("STATIC"), context("Lq/Child;", None, false)),
    ];
    for (field, access) in &cases {
        assert_eq!(
            cache.resolve(field, access),
            disabled.resolve(field, access)
        );
    }
    // Dependency requirements need only classpath resolution. No receiver or
    // source spelling work has occurred, even for rejected original contexts.
    assert_eq!(cache.stats().source_use, StageStats::default());
    assert_eq!(cache.stats().retain, StageStats::default());
    for (field, access) in &cases {
        for _ in 0..2 {
            assert_eq!(
                cache.retain_receiver(field, access),
                disabled.retain_receiver(field, access)
            );
        }
    }
    assert_eq!(cache.stats().source_use, StageStats::default());
    assert_eq!(cache.retain_receiver(&cases[0].0, &cases[0].1), Ok(false));
    assert_eq!(cache.retain_receiver(&cases[1].0, &cases[1].1), Ok(true));
    assert!(matches!(
        cache.retain_receiver(&cases[3].0, &cases[3].1),
        Err(PublicationBlocker::Resolution(FieldRequirement::Access(_)))
    ));
    for (field, access) in &cases {
        for _ in 0..2 {
            assert_eq!(
                cache.source_use(field, access),
                disabled.source_use(field, access)
            );
        }
    }
    assert_eq!(cache.stats().resolution.misses, cases.len());
    assert_eq!(cache.stats().source_use.misses, cases.len());
    assert_eq!(cache.stats().retain.misses, cases.len());
    assert_eq!(disabled.stats(), FieldUseStats::default());

    // A receiver proof for an external field doesn't require source aliases.
    // The separately requested source-use stage still retains its typed error.
    let no_aliases = JavaAliases::default();
    let no_source_cache = FieldUseCache::new(true, &[], &metadata, &no_aliases, None);
    assert_eq!(
        no_source_cache.retain_receiver(&cases[1].0, &cases[1].1),
        Ok(true)
    );
    assert!(matches!(
        no_source_cache.source_use(&cases[1].0, &cases[1].1),
        Err(PublicationBlocker::AliasUnavailable(_))
    ));
}

#[test]
fn cached_publication_records_current_owner_status_and_failures() {
    use super::super::FieldPublication;
    use crate::java_aliases::MethodKey;
    let mut base = fixture("Lp/Base;", None);
    let public = field(&mut base, "value", "I", 1);
    let private = field(&mut base, "hidden", "I", 2);
    let static_field = field(&mut base, "STATIC", "I", 9);
    let child = fixture("Lq/Child;", Some("Lp/Base;"));
    let inputs = [base, child];
    let names = aliases(&inputs);
    let index = InputIndex::new(&inputs);
    let cache = FieldUseCache::new(true, &inputs, &EmptyMetadata, &names, Some(&index));
    let collect = || FieldPublication::collect(&inputs, &mut AliasInventory::default()).unwrap();
    let mut cached = collect();
    let mut plain = collect();
    let caller = MethodKey {
        owner: "Lq/Child;".into(),
        name: "use".into(),
        parameters: vec![],
        return_type: "V".into(),
    };
    let cases = [
        (&public, context("Lq/Child;", Some("Lq/Child;"), false)),
        (&private, context("Lq/Child;", Some("Lq/Child;"), false)),
        (&static_field, context("Lq/Child;", None, true)),
        (&static_field, context("Lq/Child;", None, false)),
        (&public, context("Lp/Base;", Some("Lp/Base;"), false)),
    ];
    for initialized in [None, Some(false), Some(true)] {
        if let Some(initialized) = initialized {
            cached
                .record_owner_publication("Lp/Base;", initialized)
                .unwrap();
            plain
                .record_owner_publication("Lp/Base;", initialized)
                .unwrap();
        }
        for (key, access) in &cases {
            assert_eq!(
                cached.require_field_cached(&caller, key, access, &cache),
                plain.require_field_indexed(
                    &inputs,
                    &caller,
                    key,
                    &EmptyMetadata,
                    access,
                    Some(&index)
                )
            );
        }
        cached.audit();
        plain.audit();
        assert_eq!(format!("{cached:?}"), format!("{plain:?}"));
        assert_eq!(cached.declaration_closed(), plain.declaration_closed());
        assert_eq!(
            cached.initialization_closed(),
            plain.initialization_closed()
        );
    }
    assert!(cache.stats().resolution.hits > 0);
}
