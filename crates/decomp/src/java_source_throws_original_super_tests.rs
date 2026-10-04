use super::*;

fn key(owner: &str, result: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: "else".into(),
        parameters: vec![],
        return_type: result.into(),
    }
}
fn declaration(
    owner: &str,
    result: &str,
    flags: u32,
    throws: Option<Vec<String>>,
) -> (MethodKey, MethodMetadata) {
    (
        key(owner, result),
        MethodMetadata {
            access_flags: flags,
            exceptions: throws,
        },
    )
}
fn classes() -> BTreeMap<String, OriginalSuperClass> {
    BTreeMap::from([
        (
            "Lp/Child;".into(),
            OriginalSuperClass {
                flags: 1,
                parent: Some("Lp/Parent;".into()),
                methods: vec![],
            },
        ),
        (
            "Lp/Parent;".into(),
            OriginalSuperClass {
                flags: 1,
                parent: Some("Lp/Grand;".into()),
                methods: vec![declaration(
                    "Lp/Parent;",
                    "V",
                    1,
                    Some(vec!["Ljava/io/IOException;".into()]),
                )],
            },
        ),
        (
            "Lp/Grand;".into(),
            OriginalSuperClass {
                flags: 1,
                parent: None,
                methods: vec![declaration("Lp/Grand;", "V", 1, Some(vec![]))],
            },
        ),
    ])
}
fn selected(
    classes: &BTreeMap<String, OriginalSuperClass>,
    reference: &MethodKey,
) -> Option<(MethodKey, MethodMetadata)> {
    original_super_target("Lp/Child;", reference, |owner| classes.get(owner).cloned())
}

#[test]
fn original_super_contract_selects_exact_nonvirtual_descriptor_without_source_spelling() {
    let mut graph = classes();
    // The keyword cannot be emitted under its original spelling. Its original
    // dispatch and checked exception identity are nevertheless exact evidence.
    let (target, metadata) = selected(&graph, &key("Lp/Grand;", "V")).unwrap();
    assert_eq!(target, key("Lp/Parent;", "V"));
    assert_eq!(
        metadata.exceptions,
        Some(vec!["Ljava/io/IOException;".into()])
    );
    // A return-only companion does not alter DEX descriptor selection. Even a
    // selected bridge is original body evidence; Java publication still has
    // its separate exact source-selection guard.
    graph.get_mut("Lp/Parent;").unwrap().methods.extend([
        declaration("Lp/Parent;", "Ljava/lang/Object;", 0x1041, Some(vec![])),
        declaration("Lp/Parent;", "Ljava/lang/String;", 1, Some(vec![])),
    ]);
    for result in ["Ljava/lang/Object;", "Ljava/lang/String;"] {
        assert_eq!(
            selected(&graph, &key("Lp/Parent;", result)).unwrap().0,
            key("Lp/Parent;", result)
        );
    }
}

#[test]
fn original_super_contract_denies_incomplete_dispatch_and_genuine_access_conflicts() {
    for flags in [2, 8, 0x400, 3, 5] {
        let mut graph = classes();
        graph.get_mut("Lp/Parent;").unwrap().methods[0]
            .1
            .access_flags = flags;
        assert!(
            selected(&graph, &key("Lp/Grand;", "V")).is_none(),
            "flags={flags:#x}"
        );
    }
    for flags in [1, 4, 0x101, 0] {
        let mut graph = classes();
        graph.get_mut("Lp/Parent;").unwrap().methods[0]
            .1
            .access_flags = flags;
        assert!(
            selected(&graph, &key("Lp/Grand;", "V")).is_some(),
            "flags={flags:#x}"
        );
    }
    let mut graph = classes();
    graph.get_mut("Lp/Parent;").unwrap().methods[0].1.exceptions = None;
    assert!(selected(&graph, &key("Lp/Grand;", "V")).is_none());
    let mut graph = classes();
    let duplicate = graph["Lp/Parent;"].methods[0].clone();
    graph.get_mut("Lp/Parent;").unwrap().methods.push(duplicate);
    assert!(selected(&graph, &key("Lp/Grand;", "V")).is_none());
    for owner in ["Lp/Child;", "Lp/Foreign;"] {
        assert!(selected(&classes(), &key(owner, "V")).is_none());
    }
    let mut graph = classes();
    graph.get_mut("Lp/Parent;").unwrap().flags |= 0x200;
    assert!(selected(&graph, &key("Lp/Grand;", "V")).is_none());
    let mut graph = classes();
    graph.remove("Lp/Parent;");
    assert!(selected(&graph, &key("Lp/Grand;", "V")).is_none());
    let mut graph = classes();
    graph.get_mut("Lp/Grand;").unwrap().parent = Some("Lp/Parent;".into());
    assert!(selected(&graph, &key("Lp/Foreign;", "V")).is_none());
    let mut graph = classes();
    graph.get_mut("Lp/Child;").unwrap().parent = Some("Lq/Middle;".into());
    graph.insert(
        "Lq/Middle;".into(),
        OriginalSuperClass {
            flags: 1,
            parent: Some("Lp/Parent;".into()),
            methods: vec![],
        },
    );
    graph.get_mut("Lp/Parent;").unwrap().methods[0]
        .1
        .access_flags = 0;
    assert!(selected(&graph, &key("Lp/Grand;", "V")).is_none());
    // The original symbolic declaration's package access also stays fixed,
    // even when the nearer selected declaration is public.
    let mut graph = classes();
    graph.get_mut("Lp/Grand;").unwrap().methods[0]
        .1
        .access_flags = 0;
    graph.get_mut("Lp/Parent;").unwrap().parent = Some("Lq/Middle;".into());
    graph.insert(
        "Lq/Middle;".into(),
        OriginalSuperClass {
            flags: 1,
            parent: Some("Lp/Grand;".into()),
            methods: vec![],
        },
    );
    assert!(selected(&graph, &key("Lp/Grand;", "V")).is_none());
}

#[test]
fn original_super_contract_nonvirtual_runtime_and_external_bound_oracle() {
    use std::{fs, process::Command};
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.join("bin/javac").is_file())
        .unwrap_or_else(|| std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"));
    let dir = std::env::temp_dir().join(format!(
        "decomp-original-super-domain-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SuperDomainOracle.java"), r#"
import java.io.IOException;
class Base { void method(IOException e) throws Exception { throw new AssertionError("base"); } }
class Parent extends Base { @Override void method(IOException e) throws IOException { throw e; } }
class Child extends Parent {
  @Override void method(IOException e) throws IOException { throw new AssertionError("virtual child"); }
  void invokeParent(IOException e) throws IOException { super.method(e); }
}
public class SuperDomainOracle {
  public static void main(String[] args) throws Exception {
    IOException supplied = new IOException("identity");
    try { new Child().invokeParent(supplied); throw new AssertionError("missing throw"); }
    catch (IOException caught) { if (caught != supplied) throw new AssertionError("identity"); }
    System.out.println("exact-parent;nonvirtual;same-IOException");
  }
}
"#).unwrap();
    let compile = Command::new(home.join("bin/javac"))
        .args(["--release", "17", "SuperDomainOracle.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(home.join("bin/java"))
        .args(["-cp", ".", "SuperDomainOracle"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "exact-parent;nonvirtual;same-IOException"
    );
    fs::write(dir.join("FixedBound.java"), "class FixedExternal { void method() {} } class FixedBound extends FixedExternal { @Override void method() throws java.io.IOException {} }").unwrap();
    let negative = Command::new(home.join("bin/javac"))
        .args(["--release", "17", "FixedBound.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!negative.status.success());
    assert!(String::from_utf8_lossy(&negative.stderr).contains("does not throw IOException"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "original ModeOpenpgpCFB exact DEX super source domain; no export"]
fn original_modeopenpgpcfb_super_contract_and_source_plan() {
    let (files, aliases, catalog, external, hierarchy) = original_fixture();
    let refs = files.iter().collect::<Vec<_>>();
    let super_index = crate::java_super_resolution::SuperIndex::collect(&refs, &external).unwrap();
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[3]);
    let mut resolver = Resolver::with_catalog(&files[3], &aliases, &metadata, Some(&catalog));
    resolver.inputs = &files;
    resolver.dex_index = 3;
    resolver.external = Some(&external);
    resolver.input_owners = facts.owners.keys().cloned().collect();
    resolver.hierarchy = hierarchy.clone();
    resolver.super_index = Some(&super_index);
    let caller = method_key(&files[3], 40669).unwrap();
    let identity = OriginalMethodIdentity {
        dex: 3,
        index: 40669,
        key: caller.clone(),
    };
    let context = Context {
        resolver: &resolver,
        facts: &facts,
    };
    let contract = context
        .invoked_contract(&identity, InvokeKind::Super, MethodRef::new(40643))
        .unwrap();
    assert_eq!(
        contract.source_target,
        Some(method_key(&files[3], 40643).unwrap())
    );
    assert_eq!(contract.declared, Vec::<String>::new());
    assert!(
        resolver
            .bound_super(
                aliases.class_name(&caller.owner).unwrap(),
                MethodRef::new(40643)
            )
            .is_none()
    );
    let invalid = OriginalMethodIdentity {
        dex: 2,
        ..identity.clone()
    };
    assert!(
        context
            .invoked_contract(&invalid, InvokeKind::Super, MethodRef::new(40643))
            .is_none()
    );
    let roots = BTreeSet::from([caller.clone(), method_key(&files[2], 45058).unwrap()]);
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    for (pool, index) in [
        (3, 40669),
        (3, 40643),
        (2, 45058),
        (2, 45064),
        (2, 45088),
        (2, 45112),
    ] {
        let key = method_key(&files[pool], index).unwrap();
        println!(
            "ORIGINAL SUPER DOMAIN pool={pool} index={index} {key:?} raw={:?} source={:?} error={:?}",
            catalog.get(&key),
            plan.methods.get(&key),
            plan.unavailable.get(&key)
        );
    }
    assert!(
        plan.methods.contains_key(&caller),
        "{:?}",
        plan.unavailable.get(&caller)
    );
}
