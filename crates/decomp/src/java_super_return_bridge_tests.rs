#![allow(clippy::unwrap_used)]
use super::*;
use backend_java::type_compatibility::Declaration;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn key(owner: &str, ret: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: "getText".into(),
        parameters: vec![],
        return_type: ret.into(),
    }
}
fn method(key: MethodKey, flags: u32) -> SuperMethod {
    SuperMethod {
        key,
        metadata: MethodMetadata {
            access_flags: flags,
            exceptions: Some(vec![]),
        },
        generic: false,
    }
}
struct Fixture {
    classes: BTreeMap<String, SuperClass>,
    attested: BTreeSet<(MethodKey, MethodKey)>,
}
impl SuperMetadata for Fixture {
    fn class(&self, owner: &str) -> Option<SuperClass> {
        self.classes.get(owner).cloned()
    }
    fn attests_covariant_bridge(&self, bridge: &MethodKey, target: &MethodKey) -> bool {
        self.attested.contains(&(bridge.clone(), target.clone()))
    }
}
fn aliases(target: &MethodKey, bridge: Option<&MethodKey>) -> JavaAliases {
    let mut inventory = AliasInventory::default();
    for owner in [
        "Lfixture/Caller;",
        &target.owner,
        &target.return_type,
        "Ljava/lang/CharSequence;",
    ] {
        inventory.add_type(owner).unwrap();
    }
    inventory.add_method(target.clone()).unwrap();
    if let Some(bridge) = bridge {
        inventory.add_method(bridge.clone()).unwrap();
    }
    inventory.build(crate::java_aliases::AliasMode::Ascii)
}
fn fixture() -> (Fixture, MethodKey, MethodKey, Hierarchy) {
    let target = key("Lfixture/Base;", "Ljava/lang/String;");
    let bridge = key("Lfixture/Base;", "Ljava/lang/CharSequence;");
    let classes = BTreeMap::from([
        (
            "Lfixture/Caller;".into(),
            SuperClass {
                superclass: Some(target.owner.clone()),
                access_flags: 1,
                methods: vec![],
            },
        ),
        (
            target.owner.clone(),
            SuperClass {
                superclass: None,
                access_flags: 1,
                methods: vec![method(target.clone(), 1), method(bridge.clone(), 0x1041)],
            },
        ),
    ]);
    let mut hierarchy = Hierarchy::default();
    for (name, parent, interfaces, is_interface) in [
        ("fixture.Caller", Some("fixture.Base"), vec![], false),
        ("fixture.Base", Some("java.lang.Object"), vec![], false),
        (
            "java.lang.String",
            Some("java.lang.Object"),
            vec!["java.lang.CharSequence"],
            false,
        ),
        (
            "java.lang.CharSequence",
            Some("java.lang.Object"),
            vec![],
            true,
        ),
        ("java.lang.Object", None, vec![], false),
    ] {
        hierarchy
            .insert(Declaration {
                name: name.into(),
                superclass: parent.map(str::to_owned),
                interfaces: interfaces.into_iter().map(str::to_owned).collect(),
                is_interface,
            })
            .unwrap();
    }
    (
        Fixture {
            classes,
            attested: BTreeSet::new(),
        },
        target,
        bridge,
        hierarchy,
    )
}
#[test]
fn return_bridge_flags_alone_stay_closed_and_attested_exact_nonbridge_is_selected() {
    let (mut provider, target, bridge, hierarchy) = fixture();
    let names = aliases(&target, None);
    assert!(matches!(
        select_super("Lfixture/Caller;", &target, &provider, &names, &hierarchy),
        Err(SuperError::Selection)
    ));
    provider.attested.insert((bridge.clone(), target.clone()));
    let selection =
        select_super("Lfixture/Caller;", &target, &provider, &names, &hierarchy).unwrap();
    assert_eq!(selection.target, target);
    assert_eq!(selection.metadata.access_flags, 1);
    assert_eq!(selection.metadata.exceptions, Some(vec![]));
    assert!(matches!(
        select_super("Lfixture/Caller;", &bridge, &provider, &names, &hierarchy),
        Err(SuperError::Selection)
    ));
}
#[test]
fn return_bridge_unproved_primitive_generic_duplicate_and_source_alias_cases_stay_closed() {
    for variant in 0..10 {
        let (mut provider, mut target, mut bridge, hierarchy) = fixture();
        match variant {
            0 => bridge.return_type = "I".into(),
            1 => target.return_type = "I".into(),
            2 => target.return_type = "Lunknown/Type;".into(),
            3 => {}
            4 => {}
            5 => {}
            6 => {}
            7 => {}
            8 => {}
            _ => {}
        }
        let mut methods = vec![method(target.clone(), 1), method(bridge.clone(), 0x1041)];
        match variant {
            3 => methods[1].generic = true,
            4 => methods[1].metadata.access_flags = 0x41,
            5 => methods.push(method(key(&target.owner, "Ljava/lang/Object;"), 1)),
            6 => methods[1].metadata.access_flags |= 8,
            7 => methods[1].metadata.exceptions = None,
            8 => methods[1].metadata.access_flags |= 0x100,
            9 => methods.push(method(bridge.clone(), 0x1041)),
            _ => {}
        }
        provider.classes.get_mut(&target.owner).unwrap().methods = methods;
        provider.attested.insert((bridge.clone(), target.clone()));
        assert!(
            select_super(
                "Lfixture/Caller;",
                &target,
                &provider,
                &aliases(&target, None),
                &hierarchy
            )
            .is_err(),
            "variant {variant}"
        );
    }
    let (mut provider, target, bridge, hierarchy) = fixture();
    provider.attested.insert((bridge.clone(), target.clone()));
    let renamed = aliases(&target, Some(&bridge));
    assert_ne!(
        renamed.method_name(&target),
        Some(&MethodAlias::Ordinary("getText".into()))
    );
    assert!(matches!(
        select_super("Lfixture/Caller;", &target, &provider, &renamed, &hierarchy),
        Err(SuperError::Aliases)
    ));
}
fn java_home() -> PathBuf {
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let home = PathBuf::from(home);
        if home.join("bin/javac").is_file() {
            return home;
        }
    }
    let tools = PathBuf::from(std::env::var_os("HOME").unwrap()).join("tools");
    std::fs::read_dir(tools)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("jdk-17"))
        })
        .unwrap()
}
struct JavaFixture(PathBuf);
impl Drop for JavaFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn command(home: &Path, name: &str, args: &[&str]) -> String {
    let out = Command::new(home.join("bin").join(name))
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
#[test]
fn return_bridge_genuine_javac_nonvirtual_descriptor_and_child_override_oracle() {
    let home = java_home();
    let root = JavaFixture(
        std::env::temp_dir().join(format!("decomp-return-bridge-{}", std::process::id())),
    );
    std::fs::create_dir_all(&root.0).unwrap();
    let source = root.0.join("Oracle.java");
    std::fs::write(
        &source,
        r#"
package oracle;
interface Sequence { CharSequence getText(); }
class Base implements Sequence { public String getText() { return "base"; } }
class Child extends Base {
    @Override public String getText() { return "child"; }
    String nonbridgeSuper() { return super.getText(); }
    CharSequence originalBridgeSuper() throws Throwable {
        return (CharSequence) java.lang.invoke.MethodHandles.lookup().findSpecial(
            Base.class, "getText", java.lang.invoke.MethodType.methodType(CharSequence.class),
            Child.class).bindTo(this).invokeExact();
    }
}
public class Oracle {
    public static void main(String[] args) throws Throwable {
        Child child = new Child();
        if (!child.nonbridgeSuper().equals("base")) throw new AssertionError();
        if (!child.originalBridgeSuper().equals("child")) throw new AssertionError();
        if (!((Sequence)child).getText().equals("child")) throw new AssertionError();
        System.out.println("nonbridge=base;bridge=child;virtual=child");
    }
}
"#,
    )
    .unwrap();
    let dir = root.0.to_str().unwrap();
    let file = source.to_str().unwrap();
    command(&home, "javac", &["--release", "17", "-d", dir, file]);
    let output = command(&home, "java", &["-cp", dir, "oracle.Oracle"]);
    assert_eq!(output.trim(), "nonbridge=base;bridge=child;virtual=child");
    let code = command(
        &home,
        "javap",
        &[
            "-classpath",
            dir,
            "-c",
            "-s",
            "-p",
            "oracle.Child",
            "oracle.Base",
        ],
    );
    assert!(code.contains("invokespecial") && code.contains("Base.getText:()Ljava/lang/String;"));
    assert!(code.contains("invokevirtual") && code.contains("Method getText:()Ljava/lang/String;"));
    let classpath = crate::java_external_metadata::Classpath::from_paths([
        home.join("jmods/java.base.jmod"),
        root.0.join("oracle/Base.class"),
        root.0.join("oracle/Child.class"),
        root.0.join("oracle/Sequence.class"),
    ])
    .unwrap();
    let target = key("Loracle/Base;", "Ljava/lang/String;");
    let bridge = key("Loracle/Base;", "Ljava/lang/CharSequence;");
    assert!(classpath.attests_covariant_bridge(&bridge, &target));
    let provider = SuperIndex::collect(&[], &classpath).unwrap();
    let mut inventory = AliasInventory::default();
    for owner in [
        "Loracle/Child;",
        "Loracle/Base;",
        "Ljava/lang/String;",
        "Ljava/lang/CharSequence;",
    ] {
        inventory.add_type(owner).unwrap();
    }
    inventory.add_method(target.clone()).unwrap();
    let aliases = inventory.build(crate::java_aliases::AliasMode::Ascii);
    let hierarchy = classpath.to_hierarchy().unwrap();
    let selected =
        select_super("Loracle/Child;", &target, &provider, &aliases, &hierarchy).unwrap();
    assert_eq!(selected.target, target);
    assert!(select_super("Loracle/Child;", &bridge, &provider, &aliases, &hierarchy).is_err());
    println!(
        "genuine javac invokespecial Base.getText:()String; original bridge special execution dispatches Child.getText and differs: {output}"
    );
}
#[test]
#[ignore = "requires explicit Android35 SDK jar"]
fn return_bridge_original_android_edittext_nonbridge_descriptor_is_selected() {
    let sdk = crate::java_external_metadata::Classpath::from_paths([PathBuf::from(
        std::env::var_os("HOME").unwrap(),
    )
    .join("Android/Sdk/platforms/android-35/android.jar")])
    .unwrap();
    let target = key("Landroid/widget/EditText;", "Landroid/text/Editable;");
    let bridge = key("Landroid/widget/EditText;", "Ljava/lang/CharSequence;");
    assert!(sdk.attests_covariant_bridge(&bridge, &target));
    struct Provider<'a>(&'a crate::java_external_metadata::Classpath);
    impl SuperMetadata for Provider<'_> {
        fn class(&self, owner: &str) -> Option<SuperClass> {
            if owner == "Lfixture/Caller;" {
                return Some(SuperClass {
                    superclass: Some("Landroid/widget/EditText;".into()),
                    access_flags: 1,
                    methods: vec![],
                });
            }
            let class = self.0.class(owner)?;
            Some(SuperClass {
                superclass: class
                    .declaration
                    .superclass
                    .as_ref()
                    .map(|s| format!("L{};", s.replace('.', "/"))),
                access_flags: class.access_flags,
                methods: class
                    .methods
                    .iter()
                    .map(|m| SuperMethod {
                        key: m.key.clone(),
                        metadata: m.metadata.clone(),
                        generic: m.signature.is_some(),
                    })
                    .collect(),
            })
        }
        fn attests_covariant_bridge(&self, b: &MethodKey, t: &MethodKey) -> bool {
            self.0.attests_covariant_bridge(b, t)
        }
    }
    let mut inventory = AliasInventory::default();
    for owner in [
        "Lfixture/Caller;",
        "Landroid/widget/EditText;",
        "Landroid/text/Editable;",
        "Ljava/lang/CharSequence;",
    ] {
        inventory.add_type(owner).unwrap();
    }
    inventory.add_method(target.clone()).unwrap();
    inventory.add_method(bridge.clone()).unwrap();
    let names = inventory.build(crate::java_aliases::AliasMode::Ascii);
    assert_eq!(
        names.method_name(&target),
        Some(&MethodAlias::Ordinary("getText".into()))
    );
    let mut hierarchy = sdk.to_hierarchy().unwrap();
    hierarchy
        .insert(Declaration {
            name: "fixture.Caller".into(),
            superclass: Some("android.widget.EditText".into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    let selected = select_super(
        "Lfixture/Caller;",
        &target,
        &Provider(&sdk),
        &names,
        &hierarchy,
    )
    .unwrap();
    assert_eq!(selected.target, target);
    assert_eq!(selected.metadata.exceptions, Some(vec![]));
    assert!(
        select_super(
            "Lfixture/Caller;",
            &bridge,
            &Provider(&sdk),
            &names,
            &hierarchy
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires preserved original Samsung APK, Android35 SDK and Flip5 framework receipt"]
fn return_bridge_original_samsung_appcompat_gettext_provider_contracts() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let sdk = crate::java_external_metadata::Classpath::from_paths([
        java_home().join("jmods/java.base.jmod"),
        home.join("Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let files = dex_format::apk::parse_apk_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let dex = &files[0];
    let caller = crate::java_members::source_closure::method_key(dex, 8264).unwrap();
    assert_eq!(caller.owner, "Landroidx/appcompat/widget/j;");
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx).as_deref() == Some(&caller.owner))
        .unwrap();
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .virtual_methods
        .iter()
        .find(|m| m.method_idx == 8264)
        .unwrap();
    let code = method.code.as_ref().unwrap();
    let decoded = dex_decompiler::decode_method(code).unwrap();
    let invocation = &decoded[0];
    assert_eq!(invocation.offset, 0);
    assert!(
        matches!(&invocation.opcode, dex_decompiler::Opcode::Supported(info) if info.opcode == 0x6f)
    );
    assert_eq!(invocation.pool_index, Some(4578));
    let target = crate::java_members::source_closure::method_key(dex, 4578).unwrap();
    assert_eq!(
        target,
        key("Landroid/widget/EditText;", "Landroid/text/Editable;")
    );
    let bridge = key("Landroid/widget/EditText;", "Ljava/lang/CharSequence;");
    let refs = files.iter().collect::<Vec<_>>();
    let index = SuperIndex::collect(&refs, &sdk).unwrap();
    let mut inventory = AliasInventory::default();
    for file in &files {
        for ty in &file.type_ids {
            inventory
                .add_type(file.string(ty.descriptor_idx).unwrap())
                .unwrap();
        }
        for i in 0..file.method_ids.len() {
            inventory
                .add_method(
                    crate::java_members::source_closure::method_key(file, i as u32).unwrap(),
                )
                .unwrap();
        }
    }
    inventory_ancestors(&files, &index, &mut inventory).unwrap();
    let aliases = inventory.build(crate::java_aliases::AliasMode::Ascii);
    assert_eq!(
        aliases.method_name(&target),
        Some(&MethodAlias::Ordinary("getText".into()))
    );
    let mut hierarchy = sdk.to_hierarchy().unwrap();
    hierarchy
        .insert(Declaration {
            name: dotted(&caller.owner).unwrap(),
            superclass: Some(dotted(&descriptor(dex, class.superclass_idx).unwrap()).unwrap()),
            interfaces: class
                .interfaces
                .as_ref()
                .map(|list| {
                    list.types
                        .iter()
                        .map(|t| dotted(&descriptor(dex, u32::from(*t)).unwrap()).unwrap())
                        .collect()
                })
                .unwrap_or_default(),
            is_interface: false,
        })
        .unwrap();
    let selected = select_super(&caller.owner, &target, &index, &aliases, &hierarchy).unwrap();
    assert_eq!(selected.target, target);
    assert_eq!(selected.metadata.access_flags, 1);
    assert_eq!(selected.metadata.exceptions, Some(vec![]));
    assert!(select_super(&caller.owner, &bridge, &index, &aliases, &hierarchy).is_err());
    let device = crate::java_external_metadata::DexMetadata::from_path(
        "/tmp/opencode/samsung-framework-metadata/framework.jar",
    )
    .unwrap();
    let runtime = crate::java_external_metadata::MetadataChain::new(vec![
        crate::java_external_metadata::MetadataProvider::Dex(&device),
        crate::java_external_metadata::MetadataProvider::Jvm(&sdk),
    ]);
    // The selected runtime owner independently attests its genuine DEX bytes;
    // the shadowed SDK is never borrowed to fill missing runtime evidence.
    assert!(runtime.attests_covariant_bridge(&bridge, &target));
    let runtime_index = SuperIndex::collect(&refs, &runtime).unwrap();
    let runtime_selected =
        select_super(&caller.owner, &target, &runtime_index, &aliases, &hierarchy).unwrap();
    assert_eq!(runtime_selected.target, target);
    assert!(select_super(&caller.owner, &bridge, &runtime_index, &aliases, &hierarchy).is_err());
    println!(
        "original classes.dex #8264 invoke-super@0 ref4578: SDK selected {:?}; runtime DEX exact byte-derived companion proof selected",
        selected.target
    );
}
