use super::*;
struct GuardMetadata {
    base: Metadata,
}
impl RegionMetadata for GuardMetadata {
    fn region_scope(&self) -> &scopes::RegionScope {
        self.base.region_scope()
    }
    fn hierarchy(&self) -> &Hierarchy {
        self.base.hierarchy()
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            if key == "io" {
                "java.io.IOException"
            } else {
                "java.sql.SQLException"
            }
            .into(),
        ))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
}
struct GuardResolver(bool);
impl JavaResolver for GuardResolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "RegionRuntime".into(),
            name: if r == MethodRef::new(0) {
                "probe"
            } else {
                "log"
            }
            .into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: if r == MethodRef::new(0) {
                vec![]
            } else {
                vec![JavaType::Class("java.lang.Throwable".into())]
            },
            throws: if self.0 && r == MethodRef::new(0) {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r == ClassRef::new(1) {
                "java.io.IOException"
            } else {
                "java.sql.SQLException"
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        guard_hierarchy().assignable(a, b) == crate::type_compatibility::Compatibility::Assignable
    }
}
fn guard_hierarchy() -> Hierarchy {
    let mut h = hierarchy();
    for name in ["java.io.IOException", "java.sql.SQLException"] {
        h.insert(Declaration {
            name: name.into(),
            superclass: Some("java.lang.Exception".into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}
fn guard_fixture() -> (MethodIr<SsaForm>, GuardMetadata) {
    let mut b = Builder::new();
    let entry = b.cfg.entry();
    b.set(
        entry,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(0),
            args: vec![],
        },
    );
    let normal = b.block();
    b.link(entry, normal);
    let mut catches = vec![];
    for (key, class) in [("io", 1), ("sql", 2)] {
        let capture = b.block();
        let caught = b.value(capture, ManagedType::Reference(ClassRef::new(class)));
        b.set(
            capture,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: EffectFlags::NONE,
            },
        );
        let log = b.block();
        b.set(
            log,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![caught],
            },
        );
        b.link(capture, log);
        let end = b.block();
        b.link(log, end);
        b.edge(entry, capture, EdgeKind::Exception(key.into()));
        catches.push(scopes::Catch {
            key: key.into(),
            entry: capture,
        });
    }
    (
        b.finish(),
        GuardMetadata {
            base: Metadata {
                scope: scopes::RegionScope {
                    protected: vec![entry],
                    catches,
                    continuation: Some(normal),
                },
                hierarchy: guard_hierarchy(),
                all: false,
            },
        },
    )
}
fn guard_method() -> JavaMethod {
    JavaMethod {
        owner: "RegionRuntime".into(),
        name: "generated".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: vec![],
    }
}
#[test]
fn zero_checked_contract_guard_preserves_foreign_throwables_and_priority_jdk() {
    let (ir, md) = guard_fixture();
    let output = emit_method_region(
        &ir,
        &guard_method(),
        &DeclarationBindings::default(),
        &GuardResolver(false),
        &md,
    );
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}")
    };
    assert!(source.contains("catch (java.lang.Exception"));
    assert!(
        source.find("instanceof java.io.IOException").unwrap()
            < source.find("instanceof java.sql.SQLException").unwrap()
    );
    assert!(!source.contains("if (false)"));
    run_java(&format!(
        r#"public class RegionRuntime {{
      static int mode,effects,logs;static Throwable seen;
      static final java.io.IOException io=new java.io.IOException("io");
      static final java.sql.SQLException sql=new java.sql.SQLException("sql");
      static final RuntimeException runtime=new RuntimeException("runtime");
      static final Error error=new Error("error");
      static final Exception foreign=new Exception("foreign");
      @SuppressWarnings("unchecked") static <T extends Throwable> void boundaryThrow(Throwable t) throws T {{throw (T)t;}}
      static void probe(){{effects++;switch(mode){{case 1:boundaryThrow(io);break;case 2:boundaryThrow(sql);break;case 3:boundaryThrow(runtime);break;case 4:boundaryThrow(error);break;case 5:boundaryThrow(foreign);break;}}}}
      static void log(Throwable t){{logs++;seen=t;}}
      {source}
      public static void main(String[] args){{for(mode=0;mode<6;mode++){{effects=logs=0;seen=null;Throwable escaped=null;try{{generated();}}catch(Throwable t){{escaped=t;}}
        if(effects!=1||logs!=(mode==1||mode==2?1:0))throw new AssertionError("effects");
        if(mode==1&&seen!=io||mode==2&&seen!=sql||mode==3&&escaped!=runtime||mode==4&&escaped!=error||mode==5&&escaped!=foreign)throw new AssertionError("identity");
      }}System.out.println("PASS precise rethrow identity and priority");}}
    }}"#
    ));
}
#[test]
fn declared_checked_contract_and_missing_hierarchy_keep_direct_path() {
    let (ir, mut md) = guard_fixture();
    let output = emit_method_region(
        &ir,
        &guard_method(),
        &DeclarationBindings::default(),
        &GuardResolver(true),
        &md,
    );
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}")
    };
    assert!(!source.contains("catch (java.lang.Exception"));
    assert!(source.contains("catch (java.io.IOException"));
    md.base.hierarchy = Hierarchy::default();
    assert!(!checked_guards::eligible(
        &ir,
        &[RegionStep::Instruction(
            *ir.cfg().block(ir.cfg().entry()).unwrap().payload()
        )],
        &[JavaType::Class("java.io.IOException".into())],
        &md.base.hierarchy,
        &GuardResolver(false)
    ));
}

#[test]
fn javac_precise_rethrow_requires_immutable_parameter_and_zero_checked_contract() {
    let root = std::env::temp_dir().join(format!(
        "decomphify-checked-guard-negatives-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    for (name, source, diagnostic) in [
        (
            "NeverThrown",
            r#"public class NeverThrown {static void operation() {} static void test(){try{operation();}catch(java.io.IOException e){}}}"#,
            "never thrown",
        ),
        (
            "TouchedParameter",
            r#"public class TouchedParameter {static void operation() {} static void test(){try{operation();}catch(Exception e){e=new Exception();throw e;}}}"#,
            "unreported exception Exception",
        ),
        (
            "CheckedOperation",
            r#"public class CheckedOperation {static void operation() throws java.io.IOException {} static void test(){try{operation();}catch(Exception e){if(e instanceof java.sql.SQLException){}else{throw e;}}}}"#,
            "unreported exception IOException",
        ),
    ] {
        std::fs::write(root.join(format!("{name}.java")), source).unwrap();
        let output = std::process::Command::new("javac")
            .arg(format!("{name}.java"))
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_operation_reference_cast_and_cleanup_do_not_enable_guard() {
    struct Missing;
    impl JavaResolver for Missing {
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            None
        }
        fn class(&self, _: ClassRef) -> Option<String> {
            None
        }
        fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    let (ir, md) = guard_fixture();
    let entry = *ir.cfg().block(ir.cfg().entry()).unwrap().payload();
    let types = [JavaType::Class("java.io.IOException".into())];
    assert!(!checked_guards::eligible(
        &ir,
        &[RegionStep::Instruction(entry)],
        &types,
        md.hierarchy(),
        &Missing
    ));
    assert!(!checked_guards::eligible(
        &ir,
        &[RegionStep::Finally {
            body: vec![],
            cleanup: vec![]
        }],
        &types,
        md.hierarchy(),
        &GuardResolver(false)
    ));
    let mut b = Builder::new();
    let block = b.cfg.entry();
    let source = b.incoming(ManagedType::Reference(ClassRef::new(1)));
    let cast = b.value(block, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        block,
        ManagedInsn::Cast {
            result: cast,
            value: source,
            target: managed_ir::refs::TypeRef::new(1),
        },
    );
    let ir = b.finish();
    let id = *ir.cfg().block(ir.cfg().entry()).unwrap().payload();
    assert!(!checked_guards::eligible(
        &ir,
        &[RegionStep::Instruction(id)],
        &types,
        md.hierarchy(),
        &GuardResolver(false)
    ));
}
