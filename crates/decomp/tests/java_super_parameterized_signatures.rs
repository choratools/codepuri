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
fn parameterized_reference_and_raw_generic_ancestry_preserve_super_target() {
    let root = std::env::temp_dir().join(format!(
        "decomphify-super-signature-wave26-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = r#"import java.util.*; public class GenericProof {
 public static class Base<E>{public Object seen;public boolean put(E v){seen=v;return true;}public int accept(List<E> v){return v.size();}}
 public static class Mid<E> extends Base<E>{}
 public static class Raw extends Mid {boolean probe(Object v){return super.put((Object)v);}int list(List v){return super.accept((List)v);}}
 public static class Bound extends Base<String>{}
 public static class BoundChild extends Bound{}
 public static class MethodGeneric {public <M> boolean put(M v){return true;}}
 public static class MethodChild extends MethodGeneric{}
 public static class RawList extends AbstractList {ArrayList values=new ArrayList();public Object get(int i){return values.get(i);}public int size(){return values.size();}public void add(int i,Object v){values.add(i,v);}boolean first(Collection v){return super.addAll((Collection)v);}boolean indexed(int i,Collection v){return super.addAll((int)i,(Collection)v);}}
 public static void main(String[]args){Raw r=new Raw();Object v=new Object();if(!r.probe(v)||r.seen!=v||r.list(Arrays.asList(1,2))!=2)throw new AssertionError();RawList l=new RawList();l.first(Arrays.asList("a","b"));l.indexed(1,Arrays.asList("z"));if(!l.toString().equals("[a, z, b]"))throw new AssertionError();}
 }"#;
    std::fs::write(root.join("GenericProof.java"), source).unwrap();
    let jdk = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let run = |bin: &str, args: &[&str]| {
        let o = Command::new(jdk.join("bin").join(bin))
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    run("javac", &["--release", "17", "GenericProof.java"]);
    run("java", &["-Xverify:all", "GenericProof"]);
    let code = run("javap", &["-c", "-p", "GenericProof$RawList"]);
    assert!(
        code.contains("invokespecial")
            && code.contains("(Ljava/util/Collection;)Z")
            && code.contains("(ILjava/util/Collection;)Z")
    );
    let mut cp = Classpath::from_paths(
        std::fs::read_dir(&root)
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|p| p == "class")),
    )
    .unwrap();
    cp.add_path(jdk.join("jmods/java.base.jmod")).unwrap();
    use decomp::java_external_metadata::{ExternalMetadata, MetadataChain, MetadataProvider};
    let shadow_root = root.join("shadow");
    std::fs::create_dir_all(&shadow_root).unwrap();
    std::fs::write(
        shadow_root.join("GenericProof.java"),
        "public class GenericProof { public static class Base {} }",
    )
    .unwrap();
    let shadow_compile = Command::new(jdk.join("bin/javac"))
        .args(["--release", "17"])
        .arg(shadow_root.join("GenericProof.java"))
        .output()
        .unwrap();
    assert!(shadow_compile.status.success());
    let shadow = Classpath::from_paths([shadow_root.join("GenericProof$Base.class")]).unwrap();
    let first = MetadataChain::new(vec![
        MetadataProvider::Jvm(&shadow),
        MetadataProvider::Jvm(&cp),
    ]);
    assert_eq!(
        first.class_generic_signature("LGenericProof$Base;"),
        Some(None)
    );
    let second = MetadataChain::new(vec![
        MetadataProvider::Jvm(&cp),
        MetadataProvider::Jvm(&shadow),
    ]);
    assert!(
        second
            .class_generic_signature("LGenericProof$Base;")
            .flatten()
            .is_some()
    );
    assert_eq!(first.class_generic_signature("LMissing;"), None);
    let inputs = [
        input("LGenericProof$Raw;", "LGenericProof$Mid;"),
        input("LGenericProof$BoundChild;", "LGenericProof$Bound;"),
        input("LGenericProof$MethodChild;", "LGenericProof$MethodGeneric;"),
        input("LGenericProof$RawList;", "Ljava/util/AbstractList;"),
    ];
    let refs = inputs.iter().collect::<Vec<_>>();
    let index = SuperIndex::collect(&refs, &cp).unwrap();
    let h = decomp::java_hierarchy::build_with_metadata(&refs, &cp).unwrap();
    let mut inv = AliasInventory::default();
    for caller in [
        "LGenericProof$Raw;",
        "LGenericProof$BoundChild;",
        "LGenericProof$MethodChild;",
        "LGenericProof$RawList;",
    ] {
        let mut next: Option<String> = Some(caller.into());
        while let Some(owner) = next {
            inv.add_type(&owner).unwrap();
            let c = index.class(&owner).unwrap();
            for m in c.methods {
                inv.add_method(m.key).unwrap();
            }
            next = c.superclass;
        }
    }
    let aliases = inv.build(AliasMode::Unicode);
    for (caller, k) in [
        (
            "LGenericProof$Raw;",
            key("LGenericProof$Base;", "put", &["Ljava/lang/Object;"], "Z"),
        ),
        (
            "LGenericProof$Raw;",
            key("LGenericProof$Base;", "accept", &["Ljava/util/List;"], "I"),
        ),
        (
            "LGenericProof$RawList;",
            key(
                "Ljava/util/AbstractCollection;",
                "addAll",
                &["Ljava/util/Collection;"],
                "Z",
            ),
        ),
        (
            "LGenericProof$RawList;",
            key(
                "Ljava/util/AbstractList;",
                "addAll",
                &["I", "Ljava/util/Collection;"],
                "Z",
            ),
        ),
    ] {
        assert_eq!(
            select_super(caller, &k, &index, &aliases, &h)
                .unwrap()
                .target,
            k
        );
    }
    for (caller, k) in [
        (
            "LGenericProof$BoundChild;",
            key("LGenericProof$Base;", "put", &["Ljava/lang/Object;"], "Z"),
        ),
        (
            "LGenericProof$MethodChild;",
            key(
                "LGenericProof$MethodGeneric;",
                "put",
                &["Ljava/lang/Object;"],
                "Z",
            ),
        ),
    ] {
        assert_eq!(
            select_super(caller, &k, &index, &aliases, &h).unwrap_err(),
            SuperError::Overload
        );
    }
    std::fs::write(root.join("Bad.java"),"class Bad extends GenericProof.Bound { boolean call(Object v){return super.put((Object)v);} }").unwrap();
    let denied = Command::new(jdk.join("bin/javac"))
        .current_dir(&root)
        .args(["--release", "17", "Bad.java"])
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("String"));
}
