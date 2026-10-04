//! Field-only cross-DEX publication contract; no shared facade integration.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use decomp::{
    java_aliases::{AliasInventory, AliasMode, FieldKey, MethodKey},
    java_external_metadata::fields::publication::{
        self, FieldPublication, FieldReceiver, FieldStatus, PublicationBlocker,
    },
    java_external_metadata::{Classpath, FieldAccessContext, FieldRequirement},
};
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, EncodedField},
    encoded_value::{EncodedValue, EncodedValueKind},
    ids::{FieldId, TypeId},
};
use std::{path::PathBuf, process::Command};
const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn string(dex: &mut DexFile, s: &str) -> u32 {
    let mut value = dex.strings[0].clone();
    value.value = s.into();
    let index = dex.strings.len() as u32;
    dex.strings.push(value);
    index
}
fn ty(dex: &mut DexFile, s: &str) -> u32 {
    let descriptor_idx = string(dex, s);
    let index = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}
fn fixture(owner: &str, parent: &str) -> DexFile {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let mut class = dex.classes[0].clone();
    class.class_idx = ty(&mut dex, owner);
    class.superclass_idx = ty(&mut dex, parent);
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
fn field(dex: &mut DexFile, name: &str, desc: &str, flags: u32) -> FieldKey {
    let type_idx = ty(dex, desc) as u16;
    let name_idx = string(dex, name);
    let index = dex.field_ids.len() as u32;
    let class_idx = dex.classes[0].class_idx as u16;
    dex.field_ids.push(FieldId {
        class_idx,
        type_idx,
        name_idx,
        span: SPAN,
    });
    let data = dex.classes[0].class_data.as_mut().unwrap();
    let fields = if flags & 8 != 0 {
        &mut data.static_fields
    } else {
        &mut data.instance_fields
    };
    fields.push(EncodedField {
        field_idx: index,
        access_flags: flags,
    });
    publication::operand_key(dex, index).unwrap()
}
fn caller(owner: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: "read".into(),
        parameters: vec![],
        return_type: "I".into(),
    }
}
fn context<'a>(
    owner: &'a str,
    receiver: Option<&'a str>,
    is_static: bool,
) -> FieldAccessContext<'a> {
    FieldAccessContext {
        caller: owner,
        receiver,
        is_static,
    }
}
fn java(name: &str) -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"))
        .join("bin")
        .join(name)
}
fn cp() -> Classpath {
    Classpath::from_paths([java("java")
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("jmods/java.base.jmod")])
    .unwrap()
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn caller_access_does_not_make_private_or_cross_package_fields_inherited() {
    let external = cp();
    for (flags, child_owner) in [(2, "Lp/Child;"), (0, "Lq/Child;")] {
        let mut base = fixture("Lp/Base;", "Ljava/lang/Object;");
        let key = field(&mut base, "value", "I", flags);
        let child = fixture(child_owner, "Lp/Base;");
        let inputs = [base, child];
        let mut inventory = AliasInventory::default();
        let mut publication = FieldPublication::collect(&inputs, &mut inventory).unwrap();
        let aliases = inventory.build(AliasMode::Ascii);
        let access = context("Lp/Base;", Some(child_owner), false);
        let resolved = publication
            .require_field(&inputs, &caller("Lp/Base;"), &key, &external, &access)
            .unwrap();
        let source =
            publication::source_use(&inputs, &resolved, &external, &access, &aliases).unwrap();
        assert_eq!(source.receiver, FieldReceiver::CastToOwner);
        assert_eq!(source.key, key);
    }
}

#[test]
fn two_dex_actual_owner_hidden_and_protected_fields_compile_and_initialize() {
    let mut base = fixture("Lp/Base;", "Ljava/lang/Object;");
    let value = field(&mut base, "value", "I", 1);
    let protected = field(&mut base, "protectedValue", "I", 4);
    let constant = field(&mut base, "CONSTANT", "I", 0x19);
    base.classes[0].static_values_off = 1;
    base.classes[0].static_values = Some(vec![EncodedValue {
        value: EncodedValueKind::Int(31),
        span: SPAN,
    }]);
    let mut child = fixture("Lq/Child;", "Lp/Base;");
    field(&mut child, "value", "Ljava/lang/String;", 1);
    // Same local index, different declarations in the two files.
    assert_ne!(
        publication::operand_key(&base, 0).unwrap(),
        publication::operand_key(&child, 0).unwrap()
    );
    let inputs = [base, child];
    let external = cp();
    let mut inventory = AliasInventory::default();
    inventory.add_type("Ljava/lang/Object;").unwrap();
    let mut catalog = FieldPublication::collect(&inputs, &mut inventory).unwrap();
    let aliases = inventory.build(AliasMode::Unicode);
    let caller = caller("Lq/Child;");
    let ctx = context(&caller.owner, Some(&caller.owner), false);
    let inherited_operand = FieldKey {
        owner: caller.owner.clone(),
        ..value.clone()
    };
    let resolved = catalog
        .require_field(&inputs, &caller, &inherited_operand, &external, &ctx)
        .unwrap();
    assert_eq!(resolved.key, value);
    let source = publication::source_use(&inputs, &resolved, &external, &ctx, &aliases).unwrap();
    assert_eq!(source.receiver, FieldReceiver::CastToOwner);
    let resolved = catalog
        .require_field(&inputs, &caller, &protected, &external, &ctx)
        .unwrap();
    assert_eq!(
        publication::source_use(&inputs, &resolved, &external, &ctx, &aliases)
            .unwrap()
            .receiver,
        FieldReceiver::Retain
    );
    let resolved = catalog
        .require_field(
            &inputs,
            &caller,
            &constant,
            &external,
            &context(&caller.owner, None, true),
        )
        .unwrap();
    assert!(resolved.metadata.constant_value.is_none());
    catalog.audit();
    assert!(!catalog.declaration_closed());
    catalog.record_owner_publication("Lp/Base;", false).unwrap();
    catalog.audit();
    assert!(catalog.declaration_closed());
    assert!(!catalog.initialization_closed());
    assert_eq!(
        catalog.unresolved[&caller],
        vec![PublicationBlocker::InitializationUnavailable(
            "Lp/Base;".into()
        )]
    );

    let dir = Temp(std::env::temp_dir().join(format!("field-publication-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let declarations = |owner| {
        catalog
            .owner_declarations(owner, &aliases)
            .unwrap()
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let prefix = catalog.encoded_prefix("Lp/Base;", &aliases).unwrap();
    assert!(prefix.contains("CONSTANT = 31;"));
    std::fs::write(dir.0.join("Base.java"), format!("package p; public class Base {{ {} {prefix} static {{ System.setProperty(\"field.publication.init\", \"yes\"); }} }}", declarations("Lp/Base;"))).unwrap();
    std::fs::write(dir.0.join("Child.java"), format!("package q; public class Child extends p.Base {{ {} public int read(Child x) {{ return (({})(x)).{} + x.protectedValue; }} public static void main(String[] a) {{ Child x = new Child(); ((p.Base)x).value=7; x.protectedValue=8; x.value=\"hidden\"; if(x.read(x)!=15) throw new AssertionError(); }} }}", declarations("Lq/Child;"), source.owner, source.name)).unwrap();
    std::fs::write(dir.0.join("Probe.java"), "public class Probe { public static void main(String[] a) { if(System.getProperty(\"field.publication.init\") != null) throw new AssertionError(\"premature init\"); if(p.Base.CONSTANT != 31 || !\"yes\".equals(System.getProperty(\"field.publication.init\"))) throw new AssertionError(\"active use missing\"); q.Child.main(a); } }").unwrap();
    let output = Command::new(java("javac"))
        .arg("-d")
        .arg(&dir.0)
        .arg(dir.0.join("Base.java"))
        .arg(dir.0.join("Child.java"))
        .arg(dir.0.join("Probe.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(java("java"))
        .arg("-cp")
        .arg(&dir.0)
        .arg("Probe")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn duplicate_owners_invalid_flags_and_pool_only_members_never_publish() {
    let mut dex = fixture("Lp/Base;", "Ljava/lang/Object;");
    let key = field(&mut dex, "x", "I", 1);
    let mut aliases = AliasInventory::default();
    assert!(matches!(
        FieldPublication::collect(&[dex.clone(), dex.clone()], &mut aliases),
        Err(FieldRequirement::Ambiguous(_))
    ));
    assert!(aliases.build(AliasMode::Unicode).field_name(&key).is_none());
    dex.classes[0].class_data.as_mut().unwrap().instance_fields[0].access_flags = 3;
    assert!(matches!(
        FieldPublication::collect(&[dex.clone()], &mut aliases),
        Err(FieldRequirement::Invalid(_))
    ));
    dex.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .instance_fields
        .clear();
    let mut publication = FieldPublication::collect(&[dex.clone()], &mut aliases).unwrap();
    assert!(publication.declarations.is_empty());
    let caller = caller("Lp/Base;");
    assert!(
        publication
            .require_field(
                &[dex],
                &caller,
                &key,
                &cp(),
                &context(&caller.owner, Some(&caller.owner), false)
            )
            .is_err()
    );
    publication.audit();
    assert!(!publication.declaration_closed());
    assert!(!publication.unresolved[&caller].is_empty());
}

#[test]
fn interface_finals_and_instance_finals_keep_explicit_partial_initialization() {
    let mut interface = fixture("Lp/Values;", "Ljava/lang/Object;");
    interface.classes[0].access_flags = 0x601;
    let constant = field(&mut interface, "N", "I", 0x19);
    interface.classes[0].static_values_off = 1;
    interface.classes[0].static_values = Some(vec![EncodedValue {
        value: EncodedValueKind::Int(7),
        span: SPAN,
    }]);
    let mut ordinary = fixture("Lp/Owner;", "Ljava/lang/Object;");
    let final_field = field(&mut ordinary, "n", "I", 0x11);
    let mut aliases = AliasInventory::default();
    let publication = FieldPublication::collect(&[interface, ordinary], &mut aliases).unwrap();
    let aliases = aliases.build(AliasMode::Unicode);
    assert_eq!(
        publication.statuses[&constant],
        FieldStatus::SourceSubsetAbsent
    );
    assert!(matches!(
        publication.owner_declarations("Lp/Values;", &aliases),
        Err(PublicationBlocker::InitializationUnavailable(_))
    ));
    assert_eq!(
        publication
            .owner_declarations("Lp/Owner;", &aliases)
            .unwrap(),
        vec![(final_field, "public final int n;".into())]
    );
    assert!(
        publication.encoded["Lp/Owner;"]
            .as_ref()
            .unwrap()
            .blank_instance_finals
            .contains(&0)
    );
}

#[test]
fn alias_hidden_protected_field_rejects_illegal_owner_cast() {
    let mut base = fixture("Lp/Base;", "Ljava/lang/Object;");
    let protected = field(&mut base, "a-b", "I", 4);
    let mut child = fixture("Lq/Child;", "Lp/Base;");
    // Distinct original names can collide after sanitization across owners.
    let mut inventory = AliasInventory::default();
    let mut preliminary = AliasInventory::default();
    preliminary.add_field(protected.clone()).unwrap();
    let alias = preliminary
        .build(AliasMode::Unicode)
        .field_name(&protected)
        .unwrap()
        .to_owned();
    field(&mut child, &alias, "I", 1);
    let inputs = [base, child];
    let mut publication = FieldPublication::collect(&inputs, &mut inventory).unwrap();
    let aliases = inventory.build(AliasMode::Unicode);
    let caller = caller("Lq/Child;");
    let ctx = context(&caller.owner, Some(&caller.owner), false);
    let external = cp();
    let resolved = publication
        .require_field(&inputs, &caller, &protected, &external, &ctx)
        .unwrap();
    assert!(matches!(
        publication::source_use(&inputs, &resolved, &external, &ctx, &aliases),
        Err(PublicationBlocker::Resolution(FieldRequirement::Access(_)))
    ));
}

#[test]
fn aliased_encoded_final_uses_original_identity_and_unsupported_body_stays_partial() {
    let mut dex = fixture("Lp/Owner;", "Ljava/lang/Object;");
    let key = field(&mut dex, "not-java", "I", 0x19);
    dex.classes[0].static_values_off = 1;
    dex.classes[0].static_values = Some(vec![EncodedValue {
        value: EncodedValueKind::Int(7),
        span: SPAN,
    }]);
    let mut inventory = AliasInventory::default();
    let publication = FieldPublication::collect(&[dex.clone()], &mut inventory).unwrap();
    let aliases = inventory.build(AliasMode::Unicode);
    let name = aliases.field_name(&key).unwrap();
    assert_ne!(name, key.name);
    assert_eq!(
        publication.encoded_prefix(&key.owner, &aliases).unwrap(),
        format!("static {{\n    {name} = 7;\n}}\n")
    );
    // A genuine native capability prevents write-absence proof for the final,
    // while the real declaration and its original flags remain publishable.
    let mut native = dex.classes[0]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .first()
        .cloned()
        .unwrap_or(dex_format::class::EncodedMethod {
            method_idx: 0,
            access_flags: 0x109,
            code_off: 0,
            code: None,
        });
    let name_idx = string(&mut dex, "nativeWork");
    let proto_idx = dex.proto_ids.len() as u16;
    let return_type_idx = ty(&mut dex, "V");
    dex.proto_ids.push(dex_format::ids::ProtoId {
        shorty_idx: 0,
        return_type_idx,
        parameters_off: 0,
        parameters: None,
        span: SPAN,
    });
    native.method_idx = dex.method_ids.len() as u32;
    native.access_flags = 0x109;
    dex.method_ids.push(dex_format::ids::MethodId {
        class_idx: dex.classes[0].class_idx as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    dex.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(native);
    let mut inventory = AliasInventory::default();
    let mut publication = FieldPublication::collect(&[dex.clone()], &mut inventory).unwrap();
    let aliases = inventory.build(AliasMode::Unicode);
    assert_eq!(
        publication
            .owner_declarations(&key.owner, &aliases)
            .unwrap()[0]
            .1,
        format!("public static final int {name};")
    );
    assert_eq!(
        publication.encoded[&key.owner],
        Err(PublicationBlocker::InitializationUnavailable(
            key.owner.clone()
        ))
    );
    publication
        .record_owner_publication(&key.owner, false)
        .unwrap();
    assert_eq!(publication.statuses[&key], FieldStatus::DeclarationOnly);
    assert!(
        publication
            .record_owner_publication(&key.owner, true)
            .is_err()
    );
    let writes = decomp::java_class_kinds::initializers::write_inventory::WriteInventory::scan(
        std::slice::from_ref(&dex),
    );
    let mut names = AliasInventory::default();
    let mut explicit = FieldPublication::collect_analyst_with_explicit_stores(
        std::slice::from_ref(&dex),
        &mut names,
        &writes,
    )
    .unwrap();
    let frozen = names.build(AliasMode::Unicode);
    assert_eq!(
        explicit.encoded_prefix(&key.owner, &frozen).unwrap(),
        format!("static {{\n    {name} = 7;\n}}\n")
    );
    // Catalog prefix evidence alone never attests the complete constructor/clinit family.
    assert_eq!(explicit.statuses[&key], FieldStatus::SourceSubsetAbsent);
    explicit
        .record_owner_publication(&key.owner, false)
        .unwrap();
    assert_eq!(explicit.statuses[&key], FieldStatus::DeclarationOnly);
    assert!(!writes.unknown_observations().is_empty());
    let mut malformed = dex.clone();
    malformed.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0]
        .access_flags = 9;
    let writes = decomp::java_class_kinds::initializers::write_inventory::WriteInventory::scan(
        std::slice::from_ref(&malformed),
    );
    let denied = FieldPublication::collect_analyst_with_explicit_stores(
        &[malformed],
        &mut AliasInventory::default(),
        &writes,
    )
    .unwrap();
    assert!(denied.encoded[&key.owner].is_err());
}

#[test]
#[ignore = "one real SamsungPass DEX, field-read closure only; no APK export"]
fn samsung_single_dex_actual_reads_close_on_android_classpath() {
    use decomp::{
        java_class_kinds::initializers::write_inventory, java_external_metadata::ExternalMetadata,
    };
    use std::io::Read;
    let apk = std::env::var_os("SAMSUNGPASS_APK")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk")
        });
    let jar = std::env::var_os("ANDROID35_JAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar")
        });
    let external = Classpath::from_paths([jar]).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(apk).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("classes.dex")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let dex = DexFile::from_bytes(&bytes).unwrap();
    let reads = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
    // This catalog contains external edges only. Input owners are not invented
    // from references and this does not claim an initialization absence proof.
    let mut publication = FieldPublication::default();
    let mut observed = std::collections::BTreeSet::new();
    for index in 0..dex.field_ids.len() {
        let key = publication::operand_key(&dex, index as u32).unwrap();
        if ![
            "Landroid/view/View;",
            "Landroid/os/Build;",
            "Landroid/os/Build$VERSION;",
            "Ljava/lang/String;",
        ]
        .contains(&key.owner.as_str())
        {
            continue;
        }
        let inventory_key = write_inventory::FieldKey {
            owner: key.owner.clone(),
            name: key.name.clone(),
            descriptor: key.descriptor.clone(),
        };
        let Some(site) = reads.known_reads(&inventory_key).and_then(|r| r.first()) else {
            continue;
        };
        let Some(metadata) = external.field(&key) else {
            continue;
        };
        if metadata.access_flags & 1 == 0 {
            continue;
        }
        let raw = dex.method_id(site.method).unwrap();
        let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
        let desc = |i| {
            dex.string(dex.type_id(i).unwrap().descriptor_idx)
                .unwrap()
                .to_owned()
        };
        let owner = dex
            .string(
                dex.type_id(u32::from(raw.class_idx))
                    .unwrap()
                    .descriptor_idx,
            )
            .unwrap();
        let caller = MethodKey {
            owner: owner.into(),
            name: dex.string(raw.name_idx).unwrap().into(),
            parameters: proto
                .parameters
                .as_ref()
                .map(|p| p.types.iter().map(|t| desc(u32::from(*t))).collect())
                .unwrap_or_default(),
            return_type: desc(proto.return_type_idx),
        };
        let resolved = publication
            .require_field(
                std::slice::from_ref(&dex),
                &caller,
                &key,
                &external,
                &context(owner, None, metadata.access_flags & 8 != 0),
            )
            .unwrap();
        assert_eq!(resolved.key, key);
        assert_eq!(publication.statuses[&key], FieldStatus::ExternalClasspath);
        observed.insert(key.owner.clone());
    }
    publication.audit();
    assert!(publication.declaration_closed());
    assert!(publication.initialization_closed());
    assert!(publication.declarations.is_empty());
    assert!(publication.required_external_fields.len() >= 3);
    assert!(observed.contains("Landroid/view/View;"));
    assert!(observed.contains("Ljava/lang/String;"));
    assert!(observed.iter().any(|s| s.starts_with("Landroid/os/Build")));
    eprintln!(
        "Samsung classes.dex actual read closure: {:?}",
        publication.required_external_fields
    );
}
