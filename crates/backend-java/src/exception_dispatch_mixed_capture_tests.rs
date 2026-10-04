//! Metadata-bound catch-all values mixed with a resumed genuine checkcast.
#![allow(clippy::unwrap_used)]
use super::*;
use managed_ir::refs::TypeRef;

struct R;
impl JavaResolver for R {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "java.lang.Object",
                1 => "java.lang.Throwable",
                _ => return None,
            }
            .into(),
        )
    }
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        (r.index() == 1).then(|| JavaType::Class("java.lang.Throwable".into()))
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(9)
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "MixedCatch".into(),
            name: "raise".into(),
            is_static: true,
            parameters: vec![JavaType::Class("java.lang.Object".into())],
            return_type: JavaType::Void,
            throws: vec!["java.lang.Throwable".into()],
        })
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        hierarchy().assignable(a, b) == Compatibility::Assignable
    }
}
struct M(scopes::RegionScope);
impl RegionMetadata for M {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.0
    }
    fn hierarchy(&self) -> &Hierarchy {
        static H: std::sync::OnceLock<Hierarchy> = std::sync::OnceLock::new();
        H.get_or_init(hierarchy)
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.Throwable".into()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        true
    }
}
#[derive(Clone, Copy)]
enum Bad {
    None,
    NormalEntry,
    Opaque,
    OrdinaryObject,
    WrongCast,
}
fn mixed(bad: Bad) -> (MethodIr<SsaForm>, Vec<M>, DeclarationBindings, ValueId) {
    let mut b = Builder::new();
    let obj = b.input(ManagedType::Reference(ClassRef::new(0)));
    let resume = b.input(ManagedType::Boolean);
    let second = b.input(ManagedType::Boolean);
    let start = b.cfg.entry();
    let cast = b.block();
    let choice = b.block();
    let yes = b.edge(start, cast, EdgeKind::BranchTrue);
    let no = b.edge(start, choice, EdgeKind::BranchFalse);
    b.set(
        start,
        ManagedInsn::Branch {
            condition: Some(resume),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let cv = b.value(cast, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        cast,
        ManagedInsn::Cast {
            result: cv,
            value: obj,
            target: TypeRef::new(if matches!(bad, Bad::WrongCast) { 0 } else { 1 }),
        },
    );
    let a = b.block();
    let c = b.block();
    let yes = b.edge(choice, a, EdgeKind::BranchTrue);
    let no = b.edge(choice, c, EdgeKind::BranchFalse);
    b.set(
        choice,
        ManagedInsn::Branch {
            condition: Some(second),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let join = b.block();
    let phi = b.value(join, ManagedType::Unknown);
    let mut inputs = vec![(
        b.link(cast, join),
        if matches!(bad, Bad::OrdinaryObject) {
            obj
        } else {
            cv
        },
    )];
    let normal = b.block();
    b.set(normal, ManagedInsn::Return { value: None });
    let mut metadata = vec![];
    for site in [a, c] {
        b.set(
            site,
            ManagedInsn::Invoke {
                kind: managed_ir::ops::InvokeKind::Static,
                result: None,
                method: MethodRef::new(0),
                args: vec![obj],
            },
        );
        b.link(site, normal);
        let handler = b.block();
        let captured = b.value(handler, ManagedType::Reference(ClassRef::new(0)));
        b.edge(
            site,
            handler,
            if matches!(bad, Bad::NormalEntry) {
                EdgeKind::Unconditional
            } else {
                EdgeKind::Exception("all".into())
            },
        );
        b.set(
            handler,
            ManagedInsn::Opaque {
                family_tag: if matches!(bad, Bad::Opaque) {
                    9
                } else {
                    0x8002
                },
                reads: vec![],
                writes: vec![captured],
                effects: EffectFlags::NONE,
            },
        );
        let copy = b.block();
        let copied = b.value(copy, ManagedType::Reference(ClassRef::new(0)));
        b.set(
            copy,
            ManagedInsn::Move {
                result: copied,
                value: captured,
            },
        );
        b.link(handler, copy);
        inputs.push((b.link(copy, join), copied));
        metadata.push(M(scopes::RegionScope {
            protected: vec![site],
            catches: vec![scopes::Catch {
                key: "all".into(),
                entry: handler,
            }],
            continuation: None,
        }));
    }
    b.set(
        join,
        ManagedInsn::Phi {
            result: phi,
            inputs,
        },
    );
    let thrown = b.block();
    b.set(thrown, ManagedInsn::Throw { value: phi });
    b.link(join, thrown);
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        metadata,
        DeclarationBindings {
            parameters: vec![Some(obj), Some(resume), Some(second)],
            ..Default::default()
        },
        phi,
    )
}
fn generated(bad: Bad) -> SourceResult {
    let (ir, md, bindings, _) = mixed(bad);
    let method = JavaMethod {
        owner: "MixedCatch".into(),
        name: "generated".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![
            JavaType::Class("java.lang.Object".into()),
            JavaType::Boolean,
            JavaType::Boolean,
        ],
        throws: vec!["java.lang.Throwable".into()],
    };
    emit_method(
        &ir,
        &method,
        &bindings,
        &R,
        &md.iter()
            .map(|m| m as &dyn RegionMetadata)
            .collect::<Vec<_>>(),
    )
}
#[test]
fn mixed_catchall_and_resumed_cast_get_only_metadata_proven_declarations() {
    let (ir, md, _, phi) = mixed(Bad::None);
    let md = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let declarations = declarations_for_scopes(&ir, &JavaType::Void, &R, &md).unwrap();
    assert_eq!(
        declarations[&phi],
        JavaType::Class("java.lang.Throwable".into())
    );
    assert_eq!(ir.value(phi).unwrap().ty(), &ManagedType::Unknown);
    assert!(
        matches!(generated(Bad::None), SourceResult::Complete { .. }),
        "{:?}",
        generated(Bad::None)
    );
}
#[test]
fn metadata_cannot_attest_normal_object_ingress_mutated_capture_or_wrong_cast() {
    for bad in [
        Bad::NormalEntry,
        Bad::Opaque,
        Bad::OrdinaryObject,
        Bad::WrongCast,
    ] {
        assert!(
            matches!(generated(bad), SourceResult::Unsupported(_)),
            "{:?}",
            generated(bad)
        );
    }
}

#[test]
fn mixed_capture_emission_matches_jvm_selected_exception_identity_and_effect_order() {
    let SourceResult::Complete { source, .. } = generated(Bad::None) else {
        panic!("{:?}", generated(Bad::None));
    };
    let dir = std::env::temp_dir().join(format!("mixed-catch-original-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = format!(
        r#"public class MixedCatch {{
static String log="";
static void raise(Object x) throws Throwable {{log+="r"; if(x!=null)throw(Throwable)x;}}
static void original(Object x,boolean resume,boolean second)throws Throwable {{
 if(resume)throw(Throwable)x;
 if(second){{try{{raise(x);}}catch(Throwable t){{throw t;}}}}
 else{{try{{raise(x);}}catch(Throwable t){{throw t;}}}}
}}
{source}
static Throwable run(Object x,boolean resume,boolean second,boolean lowered){{
 try{{if(lowered)generated(x,resume,second);else original(x,resume,second);return null;}}catch(Throwable t){{return t;}}
}}
public static void main(String[]args){{
 int n=0;
 for(Object x:new Object[]{{null,new java.io.IOException("io"),new RuntimeException("rt"),new Error("error"),new Object()}})
 for(boolean resume:new boolean[]{{false,true}})for(boolean second:new boolean[]{{false,true}}){{
 log="";Throwable before=run(x,resume,second,false);String expected=log;
 log="";Throwable after=run(x,resume,second,true);
 if(!expected.equals(log))throw new AssertionError("effect order");
 if(x instanceof Throwable && (before!=x||after!=x))throw new AssertionError("identity");
 if(before==null?after!=null:after==null||before.getClass()!=after.getClass())throw new AssertionError("exception class");
 n++;
 }}System.out.print("mixed-catch-ok:"+n);
}}
}}"#
    );
    std::fs::write(dir.join("MixedCatch.java"), text).unwrap();
    for home in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let compile = std::process::Command::new(format!("{home}/bin/javac"))
            .current_dir(&dir)
            .args(["--release", "17", "MixedCatch.java"])
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new(format!("{home}/bin/java"))
            .current_dir(&dir)
            .args(["-Xverify:all", "-cp", ".", "MixedCatch"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout), "mixed-catch-ok:20");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
