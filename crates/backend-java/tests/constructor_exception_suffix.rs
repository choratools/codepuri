//! Exact constructor chain remains outside original cleanup scopes.
#![allow(clippy::unwrap_used)]
use backend_java::type_compatibility::{Declaration, Hierarchy};
use backend_java::{
    constructors::*,
    exceptions::{RegionMetadata, scopes},
    *,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::{InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Meta {
    scope: scopes::RegionScope,
    hierarchy: Hierarchy,
    original: Option<backend_java::exceptions::OriginalScope>,
}
impl RegionMetadata for Meta {
    fn original_scope(&self) -> Option<&backend_java::exceptions::OriginalScope> {
        self.original.as_ref()
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.Throwable".into()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        true
    }
}
struct Proof<'a>(&'a Meta);
impl ConstructorExceptionMetadata for Proof<'_> {
    fn scopes(&self) -> Vec<&dyn RegionMetadata> {
        vec![self.0]
    }
}
struct Resolver {
    meta: Meta,
    enable: bool,
    unknown: bool,
    literal_argument: bool,
}
impl JavaResolver for Resolver {
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
            || matches!((a,b),(JavaType::Class(a),JavaType::Class(b)) if a=="java.lang.RuntimeException" && b=="java.lang.Throwable")
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if self.unknown && r.index() == 2 {
            return None;
        }
        if r.index() < 2 {
            return self.initializer(r).map(|x| x.method);
        }
        Some(JavaMethod {
            owner: "Boundary".into(),
            name: if r.index() == 2 { "body" } else { "cleanup" }.into(),
            is_static: true,
            parameters: if r.index() == 2 && self.literal_argument {
                vec![JavaType::Int, JavaType::Int]
            } else {
                vec![JavaType::Int]
            },
            return_type: JavaType::Void,
            throws: vec![],
        })
    }
}
impl ConstructorResolver for Resolver {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 0 { "Fixture" } else { "Base" }.into(),
                name: "<init>".into(),
                is_static: false,
                parameters: vec![JavaType::Int],
                return_type: JavaType::Void,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
    fn constructor_exception_metadata(
        &self,
        _: MethodRef,
        _: &MethodIr<SsaForm>,
    ) -> Option<Box<dyn ConstructorExceptionMetadata + '_>> {
        self.enable
            .then(|| Box::new(Proof(&self.meta)) as Box<dyn ConstructorExceptionMetadata>)
    }
}
#[test]
fn chain_before_try_and_normal_cleanup_outside_scope() {
    for invalid_prefix in 0..9 {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let mut incoming = |ty| {
            let mut v = None;
            insns.insert_with_key(|id| {
                let value = values.insert(Value::new(id, ty));
                v = Some(value);
                ManagedInsn::Opaque {
                    family_tag: 0x8001,
                    reads: vec![],
                    writes: vec![value],
                    effects: managed_ir::effects::EffectFlags::NONE,
                }
            });
            v.unwrap()
        };
        let receiver = incoming(ManagedType::Reference(ClassRef::new(0)));
        let p = incoming(ManagedType::Int);
        let (zero_definition, zero) = if invalid_prefix >= 4 {
            let zero_definition = insns.insert(ManagedInsn::Return { value: None });
            let zero = values.insert(Value::new(zero_definition, ManagedType::Zero));
            insns[zero_definition] = ManagedInsn::Const {
                result: zero,
                value: if invalid_prefix == 6 {
                    managed_ir::insn::Constant::Int(1)
                } else {
                    managed_ir::insn::Constant::Zero
                },
            };
            (zero_definition, zero)
        } else {
            (values[p].def(), p)
        };
        let init = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(1),
            args: vec![receiver, if invalid_prefix == 5 { zero } else { p }],
            result: None,
        });
        let body = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            method: MethodRef::new(2),
            args: if invalid_prefix >= 4 {
                vec![p, zero]
            } else {
                vec![p]
            },
            result: None,
        });
        let cleanup = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            method: MethodRef::new(3),
            args: vec![p],
            result: None,
        });
        let ret = insns.insert(ManagedInsn::Return { value: None });
        let capture = insns.insert(ManagedInsn::Return { value: None });
        let caught = values.insert(Value::new(capture, ManagedType::Unknown));
        insns[capture] = ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: managed_ir::effects::EffectFlags::MAY_THROW,
        };
        let handlercleanup = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            method: MethodRef::new(3),
            args: vec![p],
            result: None,
        });
        let throw = insns.insert(ManagedInsn::Throw { value: caught });
        let mut cfg = Cfg::new(if invalid_prefix >= 4 {
            zero_definition
        } else {
            init
        });
        let initializer_block = if invalid_prefix >= 4 {
            let block = cfg.add_block(init);
            cfg.add_edge(cfg.entry(), block, EdgeKind::Unconditional)
                .unwrap();
            block
        } else {
            cfg.entry()
        };
        let b = cfg.add_block(body);
        let c = cfg.add_block(cleanup);
        let r = cfg.add_block(ret);
        let h = cfg.add_block(capture);
        let hc = cfg.add_block(handlercleanup);
        let t = cfg.add_block(throw);
        for (from, to) in [(initializer_block, b), (b, c), (c, r), (h, hc), (hc, t)] {
            cfg.add_edge(from, to, EdgeKind::Unconditional).unwrap();
        }
        cfg.add_edge(b, h, EdgeKind::Exception("runtime".into()))
            .unwrap();
        if invalid_prefix == 2 {
            cfg.add_edge(hc, cfg.entry(), EdgeKind::Unconditional)
                .unwrap();
        }
        if invalid_prefix == 3 {
            cfg.add_edge(c, cfg.entry(), EdgeKind::Unconditional)
                .unwrap();
        }
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let mut hierarchy = Hierarchy::default();
        for (name, parent) in [
            ("java.lang.Throwable", "java.lang.Object"),
            ("java.lang.Exception", "java.lang.Throwable"),
            ("java.lang.RuntimeException", "java.lang.Exception"),
        ] {
            hierarchy
                .insert(Declaration {
                    name: name.into(),
                    superclass: Some(parent.into()),
                    interfaces: vec![],
                    is_interface: false,
                })
                .unwrap();
        }
        let mut resolver = Resolver {
            meta: Meta {
                scope: scopes::RegionScope {
                    protected: vec![b],
                    catches: vec![scopes::Catch {
                        key: "runtime".into(),
                        entry: h,
                    }],
                    continuation: Some(c),
                },
                hierarchy,
                original: if invalid_prefix >= 4 && invalid_prefix != 8 {
                    let mut offsets = std::collections::HashMap::from([
                        (init, 0),
                        (body, 3),
                        (cleanup, 6),
                        (ret, 9),
                        (capture, 12),
                        (handlercleanup, 15),
                        (throw, 18),
                    ]);
                    if invalid_prefix == 7 {
                        offsets.insert(zero_definition, 0);
                    }
                    Some(backend_java::exceptions::OriginalScope {
                        ordinal: 0,
                        start: 3,
                        end: 6,
                        handler_offset: 1,
                        catches: vec![("runtime".into(), 12)],
                        offsets,
                    })
                } else {
                    None
                },
            },
            enable: true,
            unknown: false,
            literal_argument: invalid_prefix >= 4,
        };
        let bindings = DeclarationBindings {
            receiver: Some(receiver),
            parameters: vec![Some(p)],
            names: vec![],
        };
        if invalid_prefix == 1 {
            resolver.meta.scope.catches[0].entry = ir.cfg().entry();
        }
        if invalid_prefix != 0 && invalid_prefix != 4 {
            assert!(
                plan_constructor_declaration(&ir, MethodRef::new(0), "Base", &bindings, &resolver)
                    .is_err()
            );
            continue;
        }
        let plan =
            plan_constructor_declaration(&ir, MethodRef::new(0), "Base", &bindings, &resolver)
                .unwrap();
        let input = ConstructorInput {
            reference: MethodRef::new(0),
            ir: &ir,
            bindings: &bindings,
        };
        let evidence = constructor_initialization_evidence(&input, "Base", &[], &resolver).unwrap();
        assert!(evidence.has_exception_paths);
        assert_eq!(evidence.normal_chains.len(), 1);
        assert_eq!(plan.first_statement, "super(p0);");
        assert_eq!(plan.source.matches("super(").count(), 1);
        assert!(plan.source.contains("Boundary.body"));
        assert!(plan.source.contains("Boundary.cleanup"));
        let dir = std::env::temp_dir().join(format!(
            "constructor-exception-suffix-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let java = format!(
            r#"class Base {{ Base(int mode) {{ Boundary.log += "C"; if(mode==1) throw Boundary.runtime("chain"); }} }}
class Fixture extends Base {{ {} }}
class Boundary {{ static String log=""; static Throwable thrown; static RuntimeException runtime(String message){{RuntimeException e=new IllegalStateException(message);thrown=e;return e;}} static Error error(String message){{Error e=new AssertionError(message);thrown=e;return e;}} static void body(int mode,int zero){{if(zero!=0)throw new AssertionError("zero identity");body(mode);}} static void body(int mode){{log+="B";if(mode==5)throw error("bodyError");if(mode==2||mode==4||mode==6)throw runtime("body");}} static void cleanup(int mode){{log+="R";if(mode==6||mode==7)throw error("cleanupError");if(mode==3||mode==4)throw runtime("cleanup");}}
public static void main(String[]args){{String[] traces={{"CBR","C","CBR","CBR","CBR","CBR","CBR","CBR"}};String[] failures={{"","chain","body","cleanup","cleanup","bodyError","cleanupError","cleanupError"}};for(int mode=0;mode<8;mode++){{log="";String failure="";try{{new Fixture(mode);}}catch(Throwable e){{if(e!=thrown)throw new AssertionError("exception identity");failure=e.getMessage();}}if(!log.equals(traces[mode])||!failure.equals(failures[mode]))throw new AssertionError(mode+":"+log+":"+failure);}}}} }}"#,
            plan.source
        );
        std::fs::write(dir.join("Boundary.java"), java).unwrap();
        let compiled = std::process::Command::new("javac")
            .arg("Boundary.java")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let ran = std::process::Command::new("java")
            .args(["-cp", ".", "Boundary"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            ran.status.success(),
            "{}",
            String::from_utf8_lossy(&ran.stderr)
        );
        resolver.unknown = true;
        assert!(
            plan_constructor_declaration(&ir, MethodRef::new(0), "Base", &bindings, &resolver)
                .is_err()
        );
        resolver.unknown = false;
        resolver.enable = false;
        assert!(
            plan_constructor_declaration(&ir, MethodRef::new(0), "Base", &bindings, &resolver)
                .is_err()
        );
        resolver.enable = true;
        resolver.meta.scope.protected.push(ir.cfg().entry());
        assert!(
            plan_constructor_declaration(&ir, MethodRef::new(0), "Base", &bindings, &resolver)
                .is_err()
        );
    }
}
