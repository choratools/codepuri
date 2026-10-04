//! Exact suffix allocation effects and simultaneous original-edge Phi copies.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
struct R;
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "Fixture",
                1 => "Payload",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: if f.index() == 0 { "first" } else { "second" }.into(),
            ty: JavaType::Class("Payload".into()),
            is_static: false,
        })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() == 4 {
            Some(JavaMethod {
                owner: "Trace".into(),
                name: "argument".into(),
                parameters: vec![JavaType::Class("java.lang.Object".into())],
                return_type: JavaType::Class("java.lang.Object".into()),
                is_static: true,
                throws: vec![],
            })
        } else {
            self.initializer(m).map(|i| i.method)
        }
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: match m.index() {
                    0 => "Fixture",
                    2 => "Base",
                    3 => "Payload",
                    _ => return None,
                }
                .into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![
                        JavaType::Class("java.lang.Object".into()),
                        JavaType::Class("Payload".into()),
                    ]
                } else {
                    vec![JavaType::Class("java.lang.Object".into())]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn def(
    i: &mut InsnArena,
    v: &mut ValueArena,
    t: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut r = None;
    let id = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, t));
        r = Some(x);
        f(x)
    });
    (id, r.unwrap())
}
fn input(i: &mut InsnArena, v: &mut ValueArena, t: ManagedType) -> ValueId {
    def(i, v, t, |x| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![x],
        effects: EffectFlags::NONE,
    })
    .1
}
fn body(mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let this = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(0)));
    let context = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(2)));
    let original = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(1)));
    let chain = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(2),
        args: vec![this, context],
    });
    let (zero, z) = def(&mut i, &mut v, ManagedType::Null, |x| ManagedInsn::Const {
        result: x,
        value: Constant::Zero,
    });
    let (cmp, condition) = def(&mut i, &mut v, ManagedType::Boolean, |x| {
        ManagedInsn::Compare {
            op: managed_ir::ops::CompareOp::NotEqual,
            result: x,
            lhs: context,
            rhs: z,
        }
    });
    let branch = i.insert(ManagedInsn::Return { value: None });
    let (alloc, obj) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |x| ManagedInsn::NewInstance {
            result: x,
            class: ClassRef::new(1),
        },
    );
    let (produce, arg) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(2)),
        |x| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(x),
            method: MethodRef::new(4),
            args: vec![context],
        },
    );
    let init = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(3),
        args: vec![if mode == 3 { this } else { obj }, arg],
    });
    let jump = i.insert(ManagedInsn::Return { value: None });
    let (null, n) = def(&mut i, &mut v, ManagedType::Null, |x| ManagedInsn::Const {
        result: x,
        value: Constant::Zero,
    });
    let (phi, a) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |x| ManagedInsn::Phi {
            result: x,
            inputs: vec![],
        },
    );
    let (phi2, b) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |x| ManagedInsn::Phi {
            result: x,
            inputs: vec![],
        },
    );
    let store = i.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: a,
    });
    let store2 = i.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(1),
        value: b,
    });
    let ret = i.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(chain);
    let c = cfg.entry();
    let zz = cfg.add_block(zero);
    let q = cfg.add_block(cmp);
    let br = cfg.add_block(branch);
    let al = cfg.add_block(alloc);
    let pr = cfg.add_block(produce);
    let ini = cfg.add_block(init);
    let j = cfg.add_block(jump);
    let no = cfg.add_block(null);
    let p = cfg.add_block(phi);
    let p2 = cfg.add_block(phi2);
    let s = cfg.add_block(store);
    let s2 = cfg.add_block(store2);
    let r = cfg.add_block(ret);
    cfg.add_edge(c, zz, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(zz, q, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(q, br, EdgeKind::Unconditional).unwrap();
    let yes = cfg.add_edge(br, al, EdgeKind::BranchTrue).unwrap();
    let false_edge = cfg.add_edge(br, no, EdgeKind::BranchFalse).unwrap();
    cfg.add_edge(al, pr, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(pr, ini, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(ini, j, EdgeKind::Unconditional).unwrap();
    let edge_yes = cfg.add_edge(j, p, EdgeKind::Unconditional).unwrap();
    let edge_no = cfg.add_edge(no, p, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(p, p2, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(p2, s, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(s, s2, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(s2, r, EdgeKind::Unconditional).unwrap();
    *i.get_mut(branch).unwrap() = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge: yes,
        else_edge: Some(false_edge),
    };
    *i.get_mut(jump).unwrap() = ManagedInsn::Branch {
        condition: None,
        then_edge: if mode == 4 { edge_no } else { edge_yes },
        else_edge: None,
    };
    *i.get_mut(phi).unwrap() = ManagedInsn::Phi {
        result: a,
        inputs: vec![(edge_yes, obj), (edge_no, n)],
    };
    *i.get_mut(phi2).unwrap() = ManagedInsn::Phi {
        result: b,
        inputs: vec![
            (
                if mode == 5 { edge_no } else { edge_yes },
                if mode == 6 { a } else { original },
            ),
            (edge_no, n),
        ],
    };
    if mode == 2 {
        *i.get_mut(init).unwrap() = ManagedInsn::Move {
            result: arg,
            value: context,
        };
    }
    if mode == 7 {
        cfg.add_edge(al, p, EdgeKind::Exception("java.lang.Exception".into()))
            .unwrap();
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, i, v),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![context, original],
            names: vec![],
        },
    )
}
#[test]
fn allocations_gotos_and_cluster_inputs_remain_on_original_edges() {
    let (ir, b) = body(0);
    let p = plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).unwrap();
    assert_eq!(p.source.matches("new Payload(").count(), 1);
    assert_eq!(p.source.matches("Trace.argument(").count(), 1);
    assert!(p.source.find("new Payload(").unwrap() < p.source.find("Trace.argument(").unwrap());
    let begin = p.source.find("if (").unwrap();
    assert!(p.source[..begin].contains("super("));
    let tail = p.source.find("this.first =").unwrap();
    assert!(p.source[..tail].contains("} else {"));
    assert_eq!(p.source.matches("this.first =").count(), 1);
    assert_eq!(p.source.matches("this.second =").count(), 1);
}
#[test]
fn malformed_pairing_goto_phi_and_exception_edges_reject() {
    for mode in 2..=7 {
        let (ir, b) = body(mode);
        assert!(
            plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).is_err(),
            "accepted {mode}"
        );
    }
}
impl TrustedConstructorResolver for R {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: name.into(),
            superclass: match name {
                "Fixture" => Some("Base".into()),
                "Base" => Some("java.lang.Object".into()),
                "java.lang.Object" => None,
                _ => return None,
            },
            access_flags: 1,
        })
    }
    fn constructor_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        Some(match owner {
            "Fixture" => vec![MethodRef::new(0)],
            "Base" => vec![MethodRef::new(2)],
            _ => vec![],
        })
    }
}
#[test]
fn original_final_sites_still_count_once_per_normal_path() {
    let (ir, b) = body(0);
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &b,
    };
    let finals = [FieldRef::new(0), FieldRef::new(1)];
    let evidence = constructor_initialization_evidence(&input, "Base", &finals, &R).unwrap();
    for f in evidence.fields {
        assert_eq!(f.normal_return_counts, [1].into());
    }
    let blank = finals.map(|reference| BlankFinal {
        reference,
        writes: [(MethodRef::new(0), 1)].into(),
    });
    assert_eq!(
        plan_constructor_family("Fixture", &[input], &blank, &R)
            .unwrap()
            .len(),
        1
    );
}
fn run(tool: &str, args: &[&str], dir: &std::path::Path) -> Vec<u8> {
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let out = std::process::Command::new(format!("{home}/bin/{tool}"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
#[test]
fn jvm_original_branch_instruction_differential() {
    let (ir, b) = body(0);
    let p = plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "constructor-suffix-object-phi-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Fixture.java"),
        format!(
            "public class Fixture extends Base {{public final Payload first,second;{}}}",
            p.source
        ),
    )
    .unwrap();
    std::fs::write(dir.join("Driver.java"),r#"class Trace {static String log="";static Object argument(Object o){log+="A";if(o.equals("arg"))throw new IllegalArgumentException();return o;}}
class Payload {final Object value;Payload(Object o){Trace.log+="C";if("init".equals(o))throw new IllegalStateException();value=o;}}
class Base {Base(Object o){Trace.log+="B";if("base".equals(o))throw new UnsupportedOperationException();}}
public class Driver {public static void main(String[]args){Payload existing=new Payload("existing");for(Object mode:new Object[]{null,"ok","arg","init","base"}){Trace.log="";try{Fixture f=new Fixture(mode,existing);System.out.println(Trace.log+":"+(f.first==null?"null":f.first.value)+":"+(f.second==existing));}catch(Throwable t){System.out.println(Trace.log+":"+t.getClass().getSimpleName());}}}}"#).unwrap();
    run(
        "javac",
        &["--release", "17", "Fixture.java", "Driver.java"],
        &dir,
    );
    let after = run("java", &["-Xverify:all", "Driver"], &dir);
    std::fs::write(dir.join("Original.java"),r#"import java.nio.file.*;import java.lang.classfile.*;import java.lang.constant.*;
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassModel model=cf.parse(Path.of("Fixture.class"));ClassDesc self=ClassDesc.of("Fixture"),payload=ClassDesc.of("Payload");Files.write(Path.of("Fixture.class"),cf.transformClass(model,(cb,e)->{if(e instanceof MethodModel mm&&mm.methodName().equalsString("<init>"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label no=c.newLabel(),join=c.newLabel();c.aload(0).aload(1).invokespecial(ClassDesc.of("Base"),"<init>",MethodTypeDesc.ofDescriptor("(Ljava/lang/Object;)V"));c.aload(1).ifnull(no);c.new_(payload).dup().aload(1).invokestatic(ClassDesc.of("Trace"),"argument",MethodTypeDesc.ofDescriptor("(Ljava/lang/Object;)Ljava/lang/Object;")).invokespecial(payload,"<init>",MethodTypeDesc.ofDescriptor("(Ljava/lang/Object;)V")).astore(3);c.aload(2).astore(4);c.goto_(join);c.labelBinding(no).aconst_null().astore(3).aconst_null().astore(4);c.labelBinding(join).aload(0).aload(3).putfield(self,"first",payload).aload(0).aload(4).putfield(self,"second",payload).return_();});else mb.with(me);});else cb.with(e);}));}}"#).unwrap();
    run("javac", &["--release", "25", "Original.java"], &dir);
    run("java", &["Original"], &dir);
    let before = run("java", &["-Xverify:all", "Driver"], &dir);
    assert_eq!(before, after);
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "B:null:false\nBAC:ok:true\nBA:IllegalArgumentException\nBAC:IllegalStateException\nB:UnsupportedOperationException\n"
    );
}
