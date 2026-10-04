//! Native ordered catches exclude checked wait failures from monitor cleanup.
use super::*;

fn wait_hierarchy() -> Hierarchy {
    let mut h = hierarchy();
    h.insert(Declaration {
        name: "java.lang.InterruptedException".into(),
        superclass: Some("java.lang.Exception".into()),
        interfaces: vec![],
        is_interface: false,
    })
    .unwrap();
    h
}
struct WaitMetadata(Metadata);
impl RegionMetadata for WaitMetadata {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.0.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.0.h
    }
    fn is_catch_all(&self, key: &str) -> bool {
        key == "all"
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        if key == "io" {
            Some(JavaType::Class("java.lang.InterruptedException".into()))
        } else {
            self.0.catch_type(key)
        }
    }
}
struct WaitResolver;
impl JavaResolver for WaitResolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Resolver { missing: false }.class(r)
    }
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "WaitCleanup".into(),
            name: if reference.index() == 0 {
                "work"
            } else {
                "recovered"
            }
            .into(),
            is_static: true,
            parameters: vec![JavaType::Class("java.lang.Object".into()), JavaType::Int],
            return_type: JavaType::Int,
            throws: if reference.index() == 0 {
                vec!["java.lang.InterruptedException".into()]
            } else {
                vec![]
            },
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        wait_hierarchy().assignable(a, b) == crate::type_compatibility::Compatibility::Assignable
    }
}
fn wait_fixture(mode: u8) -> (MethodIr<SsaForm>, Vec<WaitMetadata>, DeclarationBindings) {
    let mut b = Builder::new();
    let lock = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let arg = b.incoming(ManagedType::Int);
    let foreign = b.incoming(ManagedType::Reference(ClassRef::new(1)));
    let enter = b.cfg.entry();
    b.set(enter, ManagedInsn::MonitorEnter { object: lock });
    let work = b.block();
    b.link(enter, work);
    let value = b.value(work, ManagedType::Int);
    b.set(
        work,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(value),
            method: MethodRef::new(0),
            args: vec![lock, arg],
        },
    );
    let normal_exit = b.block();
    b.link(work, normal_exit);
    b.set(normal_exit, ManagedInsn::MonitorExit { object: lock });
    let ret = b.block();
    b.link(normal_exit, ret);
    b.set(ret, ManagedInsn::Return { value: Some(value) });
    let typed = b.block();
    let typed_value = b.value(typed, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        typed,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![typed_value],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let recover = b.block();
    b.link(typed, recover);
    let recovered = b.value(recover, ManagedType::Int);
    b.set(
        recover,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(recovered),
            method: MethodRef::new(1),
            args: vec![lock, arg],
        },
    );
    let recovered_exit = b.block();
    b.link(recover, recovered_exit);
    b.set(recovered_exit, ManagedInsn::MonitorExit { object: lock });
    let recovered_ret = b.block();
    b.link(recovered_exit, recovered_ret);
    b.set(
        recovered_ret,
        ManagedInsn::Return {
            value: Some(recovered),
        },
    );
    let capture = b.block();
    let caught = b.value(capture, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        capture,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let release = b.block();
    b.link(capture, release);
    b.set(
        release,
        ManagedInsn::MonitorExit {
            object: if mode == 3 { foreign } else { lock },
        },
    );
    let throw = b.block();
    b.link(release, throw);
    b.set(
        throw,
        ManagedInsn::Throw {
            value: if mode == 2 { foreign } else { caught },
        },
    );
    let typed_key = if mode == 1 { "iae" } else { "io" };
    b.edge(work, typed, EdgeKind::Exception(typed_key.into()));
    b.edge(work, capture, EdgeKind::Exception("all".into()));
    b.edge(recover, capture, EdgeKind::Exception("all".into()));
    let md = vec![
        WaitMetadata(Metadata {
            scope: scopes::RegionScope {
                protected: vec![work],
                catches: vec![
                    scopes::Catch {
                        key: typed_key.into(),
                        entry: typed,
                    },
                    scopes::Catch {
                        key: "all".into(),
                        entry: capture,
                    },
                ],
                continuation: None,
            },
            h: wait_hierarchy(),
        }),
        WaitMetadata(Metadata {
            scope: scopes::RegionScope {
                protected: vec![recover],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: capture,
                }],
                continuation: None,
            },
            h: wait_hierarchy(),
        }),
    ];
    (
        b.finish(),
        md,
        DeclarationBindings {
            parameters: vec![Some(lock), Some(arg), Some(foreign)],
            ..Default::default()
        },
    )
}
fn wait_method() -> JavaMethod {
    JavaMethod {
        owner: "WaitCleanup".into(),
        name: "run".into(),
        is_static: true,
        parameters: vec![
            JavaType::Class("java.lang.Object".into()),
            JavaType::Int,
            JavaType::Class("java.lang.Throwable".into()),
        ],
        return_type: JavaType::Int,
        throws: vec![],
    }
}
#[test]
fn native_ordered_wait_cleanup_subtracts_only_earlier_catches() {
    let (ir, md, bindings) = wait_fixture(0);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let output = emit_method(&ir, &wait_method(), &bindings, &WaitResolver, &tables);
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}")
    };
    assert!(source.contains("catch (java.lang.InterruptedException"));
    assert!(source.contains("catch (java.lang.Throwable"));
    let dir = std::env::temp_dir().join(format!("decomphify-wait-cleanup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class WaitCleanup {{
      static final Error foreign=new AssertionError("foreign");static Object expected;
      static int work(Object lock,int mode)throws InterruptedException{{if(!Thread.holdsLock(lock))throw new AssertionError("scope");if(mode==2)throw foreign;if(mode==1)Thread.currentThread().interrupt();lock.wait(1L);return 7;}}
      static int recovered(Object lock,int mode){{if(!Thread.holdsLock(lock))throw new AssertionError("handler scope");return 9;}}
      static int original(Object lock,int mode){{synchronized(lock){{try{{return work(lock,mode);}}catch(InterruptedException e){{if(!Thread.holdsLock(lock))throw new AssertionError("interrupt scope");return 9;}}}}}}
      {source}
      public static void main(String[]args){{for(int mode=0;mode<3;mode++){{for(boolean emitted:new boolean[]{{false,true}}){{Object lock=new Object();try{{int out=emitted?run(lock,mode,null):original(lock,mode);if(out!=(mode==1?9:7))throw new AssertionError("result");}}catch(Throwable t){{if(mode!=2||t!=foreign)throw new AssertionError("identity",t);}}if(Thread.holdsLock(lock)||Thread.currentThread().isInterrupted())throw new AssertionError("release");}}}}}}
    }}"#
    );
    std::fs::write(dir.join("WaitCleanup.java"), java).unwrap();
    let home = std::env::var("DECOMPHIFY_TEST_JAVA_HOME")
        .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "WaitCleanup.java"]),
        ("java", vec!["-Xverify:all", "-cp", ".", "WaitCleanup"]),
    ] {
        let output = std::process::Command::new(std::path::Path::new(&home).join("bin").join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn native_ordered_wait_cleanup_rejects_changed_handler_object_and_lock() {
    for mode in 1..=3 {
        let (ir, md, bindings) = wait_fixture(mode);
        let tables = md
            .iter()
            .map(|m| m as &dyn RegionMetadata)
            .collect::<Vec<_>>();
        let output = emit_method(&ir, &wait_method(), &bindings, &WaitResolver, &tables);
        assert!(
            matches!(output, SourceResult::Unsupported(_)),
            "mode={mode} {output:?}"
        );
    }
}
