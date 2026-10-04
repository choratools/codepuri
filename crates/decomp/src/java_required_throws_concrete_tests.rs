#![allow(clippy::unwrap_used)]
use super::*;
use crate::java_members::external_metadata::ExternalMetadata;
use backend_java::{DeclarationBindings, JavaField, JavaMethod, SourceResult};
use dex_decompiler::{DexPasses, TypeContext};
use managed_ir::refs::{FieldRef, StringRef};

struct Context {
    parent_flags: u32,
    parent_original: Vec<String>,
    bound: Option<Vec<String>>,
    attested: bool,
    source: BTreeMap<MethodKey, Vec<String>>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            parent_flags: 1,
            parent_original: vec![],
            bound: None,
            attested: true,
            source: BTreeMap::new(),
        }
    }
}
fn identity(index: u32) -> OriginalMethodIdentity {
    OriginalMethodIdentity {
        dex: 0,
        index,
        key: MethodKey {
            owner: match index {
                0 => "LParent;",
                1 => "LChild;",
                _ => "LCaller;",
            }
            .into(),
            name: if index == 2 { "call" } else { "run" }.into(),
            parameters: if index == 2 {
                vec!["LParent;".into(), "Ljava/io/IOException;".into()]
            } else {
                vec!["Ljava/io/IOException;".into()]
            },
            return_type: "V".into(),
        },
    }
}
impl JavaResolver for Context {
    fn super_invocation(
        &self,
        caller: &str,
        reference: MethodRef,
    ) -> Option<backend_java::SuperInvocation> {
        (caller == "Child" && reference.index() == 0).then(|| backend_java::SuperInvocation {
            method: self.method(reference).unwrap(),
            access_flags: 1,
        })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let id = identity(r.index());
        Some(JavaMethod {
            owner: id
                .key
                .owner
                .trim_start_matches('L')
                .trim_end_matches(';')
                .into(),
            name: id.key.name.clone(),
            is_static: r.index() == 2,
            return_type: JavaType::Void,
            parameters: if r.index() == 2 {
                vec![
                    JavaType::Class("Parent".into()),
                    JavaType::Class("java.io.IOException".into()),
                ]
            } else {
                vec![JavaType::Class("java.io.IOException".into())]
            },
            throws: self.source.get(&id.key).cloned().unwrap_or_else(|| {
                if r.index() == 0 {
                    self.parent_original.clone()
                } else {
                    vec![]
                }
            }),
        })
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(match r.index() {
            0 => self.parent_flags,
            1 => 1,
            _ => 0xa,
        })
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "java.io.IOException",
                1 => "Parent",
                _ => "Child",
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
            || matches!((a,b), (JavaType::Class(a),JavaType::Class(b))
            if ["java.io.IOException","java.sql.SQLException"].contains(&a.as_str())
                && ["java.lang.Exception","java.lang.Throwable","java.lang.Object"].contains(&b.as_str()))
    }
}
impl ThrowsHierarchy for Context {
    fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
        [
            "java.io.IOException",
            "java.sql.SQLException",
            "java.lang.Exception",
            "java.lang.Throwable",
        ]
        .contains(&name)
        .then_some(ExceptionKind::Checked)
    }
}
impl RequiredThrowsContext for Context {
    fn invocation_contract(&self, r: MethodRef) -> Option<InvocationContract> {
        Some(InvocationContract {
            source_target: Some(identity(r.index()).key),
            declared: if r.index() == 0 {
                self.parent_original.clone()
            } else {
                vec![]
            },
        })
    }
    fn attests_concrete_override_domain(
        &self,
        i: &OriginalMethodIdentity,
        implementations: &[MethodKey],
    ) -> bool {
        self.attested && i == &identity(0) && implementations.iter().all(|k| k == &identity(1).key)
    }
    fn override_contract(&self, i: &OriginalMethodIdentity) -> OverrideContract {
        match i.index {
            0 => self
                .bound
                .as_ref()
                .map(|throws| {
                    OverrideContract::Declarations(vec![OverrideDeclaration {
                        key: MethodKey {
                            owner: "LExternal;".into(),
                            ..i.key.clone()
                        },
                        throws: throws.clone(),
                        source_target: None,
                    }])
                })
                .unwrap_or(OverrideContract::NoInheritedDeclaration),
            1 => OverrideContract::Declarations(vec![OverrideDeclaration {
                key: identity(0).key.clone(),
                throws: self.parent_original.clone(),
                source_target: Some(identity(0).key),
            }]),
            _ => OverrideContract::NoInheritedDeclaration,
        }
    }
}
struct Types(u32);
impl TypeContext for Types {
    fn initial_register_type(&self, register: u16, wide: bool) -> Option<ManagedType> {
        if wide {
            return None;
        }
        Some(ManagedType::Reference(ClassRef::new(if register == 1 {
            0
        } else if self.0 == 1 {
            2
        } else {
            1
        })))
    }
}
fn fixture(
    index: u32,
    context: &Context,
) -> (CodeItem, dex_decompiler::ssa::SsaMethod, MethodRequirements) {
    let units: Vec<u16> = match index {
        0 => vec![0x000e],
        1 => vec![0x206f, 0, 0x0010, 0x0127],
        _ => vec![0x206e, 0, 0x0010, 0x000e],
    };
    let code = CodeItem {
        registers_size: 2,
        ins_size: 2,
        outs_size: if index != 0 { 2 } else { 0 },
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|n| n.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan {
            offset: 0,
            size: units.len() as u32 * 2,
        },
    };
    let ssa = DexPasses::run_ssa_with(&code, &Types(index)).unwrap();
    let original = if index == 0 {
        context.parent_original.clone()
    } else {
        vec![]
    };
    let inspected = inspect_method(
        identity(index),
        &ssa,
        &code,
        context.method_access(MethodRef::new(index)).unwrap(),
        original,
        context,
    )
    .unwrap();
    (code, ssa, inspected)
}
#[test]
fn concrete_domain_preserves_body_original_and_propagates_recursive_source_contracts() {
    let context = Context::default();
    let (_, _, parent) = fixture(0, &context);
    let parent = attest_concrete_override_domain(parent, vec![identity(1).key], &context).unwrap();
    let (_, _, child) = fixture(1, &context);
    let (_, _, caller) = fixture(2, &context);
    let plan = derive_program(&[parent, child, caller], &context).unwrap();
    assert!(plan.unavailable.is_empty(), "{:?}", plan.unavailable);
    for index in 0..3 {
        let proof = &plan.methods[&identity(index).key];
        assert!(proof.original.is_empty());
        assert_eq!(proof.source, vec!["java.io.IOException"]);
    }
}
#[test]
fn concrete_domain_external_no_throws_and_input_annotation_bounds_stay_closed() {
    let context = Context {
        bound: Some(vec![]),
        ..Context::default()
    };
    let (_, _, parent) = fixture(0, &context);
    let parent = attest_concrete_override_domain(parent, vec![identity(1).key], &context).unwrap();
    let (_, _, child) = fixture(1, &context);
    let (_, _, caller) = fixture(2, &context);
    let plan = derive_program(&[parent, child, caller], &context).unwrap();
    assert!(plan.methods.is_empty());
    assert_eq!(
        plan.unavailable[&identity(0).key],
        ThrowsError::OverrideConflict("java.io.IOException".into())
    );
    let context = Context {
        parent_original: vec!["java.sql.SQLException".into()],
        ..Context::default()
    };
    let (_, _, parent) = fixture(0, &context);
    assert!(
        attest_concrete_override_domain(parent.clone(), vec![identity(1).key], &context).is_err()
    );
    let (_, _, child) = fixture(1, &context);
    let plan = derive_program(&[parent, child], &context).unwrap();
    assert_eq!(
        plan.methods[&identity(0).key].source,
        vec!["java.sql.SQLException"]
    );
    assert_eq!(
        plan.unavailable[&identity(1).key],
        ThrowsError::OverrideConflict("java.io.IOException".into())
    );
}
#[test]
fn concrete_domain_flags_duplicate_foreign_and_unattested_links_stay_closed() {
    for flag in [2, 8, 0x10, 0x100, 0x400] {
        let context = Context {
            parent_flags: 1 | flag,
            ..Context::default()
        };
        let (_, _, parent) = fixture(0, &context);
        assert!(attest_concrete_override_domain(parent, vec![identity(1).key], &context).is_err());
    }
    for keys in [
        vec![identity(0).key],
        vec![identity(1).key.clone(), identity(1).key],
        vec![MethodKey {
            owner: "LForeign;".into(),
            ..identity(1).key
        }],
        vec![identity(2).key],
    ] {
        let context = Context::default();
        let (_, _, parent) = fixture(0, &context);
        assert!(attest_concrete_override_domain(parent, keys, &context).is_err());
    }
    let context = Context {
        attested: false,
        ..Context::default()
    };
    let (_, _, parent) = fixture(0, &context);
    assert!(attest_concrete_override_domain(parent, vec![identity(1).key], &context).is_err());
    let context = Context::default();
    let (_, _, mut parent) = fixture(0, &context);
    parent.identity.key.name = "<init>".into();
    assert!(attest_concrete_override_domain(parent, vec![identity(1).key], &context).is_err());
}

#[test]
fn concrete_domain_genuine_emitted_methods_compile_and_preserve_exception_identity() {
    let mut context = Context::default();
    let (_, _, parent) = fixture(0, &context);
    let parent = attest_concrete_override_domain(parent, vec![identity(1).key], &context).unwrap();
    let (_, _, child) = fixture(1, &context);
    let (_, _, caller) = fixture(2, &context);
    let plan = derive_program(&[parent, child, caller], &context).unwrap();
    context.source = plan
        .methods
        .iter()
        .map(|(k, p)| (k.clone(), p.source.clone()))
        .collect();
    let mut sources = Vec::new();
    for index in 0..3 {
        let (_, ssa, _) = fixture(index, &context);
        let initial = |reg| {
            ssa.initial_registers()
                .iter()
                .find(|(_, registers)| registers.contains(&reg))
                .map(|(value, _)| *value)
        };
        let bindings = DeclarationBindings {
            receiver: if index == 2 { None } else { initial(0) },
            parameters: if index == 2 {
                vec![initial(0), initial(1)]
            } else {
                vec![initial(1)]
            },
            names: vec![],
        };
        let emitted = backend_java::emit_method_declaration(
            ssa.ir(),
            &context.method(MethodRef::new(index)).unwrap(),
            &bindings,
            &context,
        );
        let SourceResult::Complete { source, .. } = emitted else {
            panic!("{emitted:?}")
        };
        sources.push(source);
    }
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.join("bin/javac").is_file())
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("tools/jdk-17.0.13+11")
        });
    let dir = std::env::temp_dir().join(format!("decomp-concrete-domain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        "class Parent {{ {} }}\nclass Child extends Parent {{ {} }}\nclass Caller {{ {} }}\npublic class ConcreteDomainOracle {{ public static void main(String[] args) throws Exception {{ java.io.IOException original=new java.io.IOException(\"same\"); Caller.call(new Parent(),original); try {{ Caller.call(new Child(),original); throw new AssertionError(); }} catch(java.io.IOException actual) {{ if(actual!=original)throw new AssertionError(\"identity\"); }} System.out.println(\"parent=noop;child=same-exception;caller=propagates\"); }} }}",
        sources[0], sources[1], sources[2]
    );
    std::fs::write(dir.join("ConcreteDomainOracle.java"), java).unwrap();
    let compile = std::process::Command::new(home.join("bin/javac"))
        .args(["--release", "17", "ConcreteDomainOracle.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(home.join("bin/java"))
        .args(["-cp", ".", "ConcreteDomainOracle"])
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
        "parent=noop;child=same-exception;caller=propagates"
    );
    std::fs::write(
        dir.join("External.java"),
        "public class External { public void run(java.io.IOException e) {} }",
    )
    .unwrap();
    let external_compile = std::process::Command::new(home.join("bin/javac"))
        .args(["--release", "17", "External.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(external_compile.status.success());
    let genuine =
        crate::java_members::external_metadata::Classpath::from_paths(
            &[dir.join("External.class")],
        )
        .unwrap();
    let declaration = genuine
        .method(&MethodKey {
            owner: "LExternal;".into(),
            ..identity(0).key
        })
        .unwrap();
    assert_eq!(declaration.access_flags, 1);
    assert_eq!(declaration.exceptions, Some(vec![]));
    std::fs::write(
        dir.join("Parent.java"),
        format!("class Parent extends External {{ {} }}", sources[0]),
    )
    .unwrap();
    let denied = std::process::Command::new(home.join("bin/javac"))
        .args(["--release", "17", "Parent.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr)
            .contains("overridden method does not throw IOException")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
