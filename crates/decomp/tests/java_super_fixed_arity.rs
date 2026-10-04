//! Genuine classfiles prove fixed-arity super selection and raw-parent limits.
#![allow(clippy::unwrap_used)]
use decomp::{
    java_aliases::{AliasInventory, AliasMode, MethodKey},
    java_external_metadata::Classpath,
    java_super_resolution::{SuperError, SuperIndex, SuperMetadata, select_super},
};
use dex_format::{DexFile, RawSpan, ids::TypeId, strings::StringData};
use std::{path::PathBuf, process::Command};
fn input(caller: &str, parent: &str) -> DexFile {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes_named.dex"
    ))
    .unwrap();
    dex.classes.truncate(1);
    let name = dex
        .type_id(dex.classes[0].class_idx)
        .unwrap()
        .descriptor_idx;
    dex.strings[name as usize].value = caller.into();
    let span = RawSpan { offset: 0, size: 0 };
    let descriptor_idx = dex.strings.len() as u32;
    dex.strings.push(StringData {
        value: parent.into(),
        utf16_size: parent.len() as u32,
        span,
    });
    dex.classes[0].superclass_idx = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span,
    });
    dex
}
fn key(owner: &str, name: &str, parameters: &[&str], return_type: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: name.into(),
        parameters: parameters.iter().map(|p| (*p).into()).collect(),
        return_type: return_type.into(),
    }
}
#[test]
fn actual_fixed_arity_bridge_varargs_raw_parent_and_generic_limits() {
    let root = std::env::temp_dir().join(format!(
        "decomphify-super-fixed-arity-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let text = r#"public class SuperProof {
    static Object seen;static Object[] active;static int calls;
    public static class VarBase {public String text(int n){return "fixed"+n;} public String text(int n,Object...args){if(args!=active)throw new AssertionError("array identity");return "array"+n;}}
    public static class VarChild extends VarBase {String one(int n){return super.text((int)n);}String two(int n,Object[]a){return super.text((int)n,(Object[])a);}}
    public static class BridgeBase<T>{public void set(T value){throw new AssertionError("bridge base");}}
    public static class BridgeMid extends BridgeBase<Number>{@Override public void set(Number value){calls++;seen=value;}}
    public static class BridgeChild extends BridgeMid{void probe(Number v){super.set((Number)v);}}
    public static class Generic<E>{public boolean add(E value){seen=value;return true;}}
    public static class RawChild extends Generic{boolean probe(Object value){return super.add((Object)value);}}
    public static class BoundMid extends Generic<String>{}
    public static class BoundChild extends BoundMid{}
    public static class MethodGeneric{public <T> boolean add(T value){return true;}}
    public static class MethodChild extends MethodGeneric{}
    public static class ReturnBase{public Object value(){return new Object();}}
    public static class ReturnMid extends ReturnBase{@Override public String value(){return "narrow";}}
    public static class ReturnChild extends ReturnMid{}
    public static void main(String[]args){active=new Object[]{new Object()};VarChild v=new VarChild();if(!v.one(4).equals("fixed4")||!v.two(7,active).equals("array7"))throw new AssertionError();Number n=Integer.valueOf(11);new BridgeChild().probe(n);if(calls!=1||seen!=n)throw new AssertionError();Object object=new Object();if(!new RawChild().probe(object)||seen!=object)throw new AssertionError();}
    }"#;
    std::fs::write(root.join("SuperProof.java"), text).unwrap();
    let jdk = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let run = |name: &str, args: &[&str]| {
        let result = Command::new(jdk.join("bin").join(name))
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}:{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };
    run("javac", &["--release", "17", "SuperProof.java"]);
    run("java", &["-Xverify:all", "SuperProof"]);
    let mut cp = Classpath::from_paths(
        std::fs::read_dir(&root)
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|p| p == "class")),
    )
    .unwrap();
    cp.add_path(jdk.join("jmods/java.base.jmod")).unwrap();
    let files = [
        input("LSuperProof$VarChild;", "LSuperProof$VarBase;"),
        input("LSuperProof$BridgeChild;", "LSuperProof$BridgeMid;"),
        input("LSuperProof$RawChild;", "LSuperProof$Generic;"),
        input("LSuperProof$BoundChild;", "LSuperProof$BoundMid;"),
        input("LSuperProof$MethodChild;", "LSuperProof$MethodGeneric;"),
        input("LSuperProof$ReturnChild;", "LSuperProof$ReturnMid;"),
    ];
    let refs = files.iter().collect::<Vec<_>>();
    let index = SuperIndex::collect(&refs, &cp).unwrap();
    let hierarchy = decomp::java_hierarchy::build_with_metadata(&refs, &cp).unwrap();
    let mut names = AliasInventory::default();
    for file in &files {
        let owner = file
            .string(
                file.type_id(file.classes[0].class_idx)
                    .unwrap()
                    .descriptor_idx,
            )
            .unwrap();
        names.add_type(owner).unwrap();
        let mut next = index.class(owner).unwrap().superclass;
        while let Some(owner) = next {
            names.add_type(&owner).unwrap();
            let c = index.class(&owner).unwrap();
            for m in c.methods {
                names.add_method(m.key).unwrap();
            }
            next = c.superclass;
        }
    }
    let aliases = names.build(AliasMode::Unicode);
    for (caller, reference) in [
        (
            "LSuperProof$VarChild;",
            key("LSuperProof$VarBase;", "text", &["I"], "Ljava/lang/String;"),
        ),
        (
            "LSuperProof$VarChild;",
            key(
                "LSuperProof$VarBase;",
                "text",
                &["I", "[Ljava/lang/Object;"],
                "Ljava/lang/String;",
            ),
        ),
        (
            "LSuperProof$BridgeChild;",
            key(
                "LSuperProof$BridgeMid;",
                "set",
                &["Ljava/lang/Number;"],
                "V",
            ),
        ),
        (
            "LSuperProof$RawChild;",
            key("LSuperProof$Generic;", "add", &["Ljava/lang/Object;"], "Z"),
        ),
    ] {
        assert_eq!(
            select_super(caller, &reference, &index, &aliases, &hierarchy)
                .unwrap()
                .target,
            reference
        );
    }
    assert_eq!(
        select_super(
            "LSuperProof$BoundChild;",
            &key("LSuperProof$Generic;", "add", &["Ljava/lang/Object;"], "Z"),
            &index,
            &aliases,
            &hierarchy
        )
        .unwrap_err(),
        SuperError::Overload
    );
    assert_eq!(
        select_super(
            "LSuperProof$MethodChild;",
            &key(
                "LSuperProof$MethodGeneric;",
                "add",
                &["Ljava/lang/Object;"],
                "Z"
            ),
            &index,
            &aliases,
            &hierarchy
        )
        .unwrap_err(),
        SuperError::Overload
    );
    assert_eq!(
        select_super(
            "LSuperProof$BridgeChild;",
            &key(
                "LSuperProof$BridgeMid;",
                "set",
                &["Ljava/lang/Object;"],
                "V"
            ),
            &index,
            &aliases,
            &hierarchy
        )
        .unwrap_err(),
        SuperError::Overload
    );
    assert_eq!(
        select_super(
            "LSuperProof$ReturnChild;",
            &key("LSuperProof$ReturnMid;", "value", &[], "Ljava/lang/Object;"),
            &index,
            &aliases,
            &hierarchy
        )
        .unwrap_err(),
        SuperError::Selection
    );
    std::fs::write(root.join("Bad.java"),"class Bad extends SuperProof.BoundMid{boolean probe(Object value){return super.add((Object)value);}}").unwrap();
    let invalid = Command::new(jdk.join("bin/javac"))
        .args(["--release", "17", "Bad.java"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        !invalid.status.success(),
        "parameterized external intermediate must not be treated raw"
    );
    let code = run(
        "javap",
        &[
            "-c",
            "-p",
            "SuperProof$VarChild",
            "SuperProof$BridgeChild",
            "SuperProof$RawChild",
        ],
    );
    assert!(code.contains("SuperProof$VarBase.text:(I)Ljava/lang/String;"));
    assert!(code.contains("SuperProof$VarBase.text:(I[Ljava/lang/Object;)Ljava/lang/String;"));
    assert!(code.contains("SuperProof$BridgeMid.set:(Ljava/lang/Number;)V"));
    assert!(code.contains("SuperProof$Generic.add:(Ljava/lang/Object;)Z"));
    std::fs::remove_dir_all(root).unwrap();
}
