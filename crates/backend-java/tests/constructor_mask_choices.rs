//! Original nullable-mask prefixes preserve selected arguments and exact edges.
#![allow(clippy::unwrap_used)]
use backend_java::{DeclarationBindings, JavaMethod, JavaResolver, JavaType, constructors::*};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{ArithOp, CompareOp, Constant, InvokeKind, ManagedInsn},
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
                1 => "java.lang.Object",
                _ => "Marker",
            }
            .into(),
        )
    }
    fn field(&self, f: FieldRef) -> Option<backend_java::JavaField> {
        Some(backend_java::JavaField {
            owner: "Fixture".into(),
            name: if f.index() == 0 { "first" } else { "second" }.into(),
            ty: JavaType::Class("java.lang.Object".into()),
            is_static: false,
        })
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        self.initializer(m).map(|i| i.method)
    }
    fn method_access(&self, m: MethodRef) -> Option<u32> {
        Some(if m.index() == 0 { 0x1001 } else { 1 })
    }
}
impl ConstructorResolver for R {
    fn constructor_source_level(&self) -> u16 {
        25
    }
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        let object = JavaType::Class("java.lang.Object".into());
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 2 {
                    "java.lang.Object"
                } else {
                    "Fixture"
                }
                .into(),
                name: "<init>".into(),
                parameters: match m.index() {
                    0 => vec![
                        object.clone(),
                        object,
                        JavaType::Int,
                        JavaType::Class("Marker".into()),
                    ],
                    1 => vec![object.clone(), object],
                    2 => vec![],
                    _ => return None,
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: (m.index() == 0).then_some(MethodRef::new(1)),
        })
    }
}
impl TrustedConstructorResolver for R {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: name.into(),
            superclass: match name {
                "Fixture" => Some("java.lang.Object".into()),
                "java.lang.Object" => None,
                _ => return None,
            },
            access_flags: 1,
        })
    }
    fn constructor_access(&self, m: MethodRef) -> Option<u32> {
        self.method_access(m)
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        Some(match owner {
            "Fixture" => vec![MethodRef::new(0), MethodRef::new(1)],
            "java.lang.Object" => vec![MethodRef::new(2)],
            _ => return None,
        })
    }
}
fn def(
    i: &mut InsnArena,
    v: &mut ValueArena,
    t: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut result = None;
    let id = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, t));
        result = Some(x);
        f(x)
    });
    (id, result.unwrap())
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
// Modes mutate one independent original evidence obligation.
fn fixture(mode: u8) -> (MethodIr<SsaForm>, DeclarationBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let this = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(0)));
    let a = input(
        &mut i,
        &mut v,
        if mode == 12 {
            ManagedType::Float
        } else {
            ManagedType::Reference(ClassRef::new(1))
        },
    );
    let b = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(1)));
    let mask = input(&mut i, &mut v, ManagedType::Int);
    let (zero, z) = def(&mut i, &mut v, ManagedType::Zero, |x| ManagedInsn::Const {
        result: x,
        value: if mode == 3 {
            Constant::Int(7)
        } else {
            Constant::Zero
        },
    });
    let mut cfg = Cfg::new(if mode == 16 {
        v.get(a).unwrap().def()
    } else {
        zero
    });
    let mut previous = cfg.entry();
    if mode == 16 {
        for instruction in [
            v.get(b).unwrap().def(),
            v.get(mask).unwrap().def(),
            v.get(this).unwrap().def(),
            zero,
        ] {
            let next = cfg.add_block(instruction);
            cfg.add_edge(previous, next, EdgeKind::Unconditional)
                .unwrap();
            previous = next;
        }
    }
    let mut arguments = vec![];
    for (position, original) in [a, b].into_iter().enumerate() {
        let (literal, bit) = def(&mut i, &mut v, ManagedType::Int, |x| ManagedInsn::Const {
            result: x,
            value: Constant::Int(if mode == 1 && position == 0 {
                4
            } else {
                1 << position
            }),
        });
        let (and, tested) = def(&mut i, &mut v, ManagedType::Int, |x| {
            ManagedInsn::Arithmetic {
                result: x,
                op: ArithOp::And,
                lhs: if mode == 11 { z } else { mask },
                rhs: bit,
            }
        });
        let (compare, condition) = def(&mut i, &mut v, ManagedType::Boolean, |x| {
            ManagedInsn::Compare {
                result: x,
                op: if mode == 2 {
                    CompareOp::Equal
                } else {
                    CompareOp::NotEqual
                },
                lhs: tested,
                rhs: z,
            }
        });
        let branch = i.insert(ManagedInsn::Return { value: None });
        let (copy, default) = def(&mut i, &mut v, ManagedType::Zero, |x| ManagedInsn::Move {
            result: x,
            value: if mode == 7 { this } else { z },
        });
        let (phi, result) = def(
            &mut i,
            &mut v,
            ManagedType::Reference(ClassRef::new(1)),
            |x| ManagedInsn::Phi {
                result: x,
                inputs: vec![],
            },
        );
        let blocks = [literal, and, compare, branch].map(|id| cfg.add_block(id));
        cfg.add_edge(previous, blocks[0], EdgeKind::Unconditional)
            .unwrap();
        for pair in blocks.windows(2) {
            cfg.add_edge(pair[0], pair[1], EdgeKind::Unconditional)
                .unwrap();
        }
        let arm = cfg.add_block(copy);
        let join = cfg.add_block(phi);
        let yes = cfg.add_edge(blocks[3], arm, EdgeKind::BranchTrue).unwrap();
        let no = cfg
            .add_edge(blocks[3], join, EdgeKind::BranchFalse)
            .unwrap();
        let incoming = cfg.add_edge(arm, join, EdgeKind::Unconditional).unwrap();
        *i.get_mut(branch).unwrap() = ManagedInsn::Branch {
            condition: Some(condition),
            then_edge: yes,
            else_edge: Some(no),
        };
        *i.get_mut(phi).unwrap() = ManagedInsn::Phi {
            result,
            inputs: if mode == 4 {
                vec![(incoming, default)]
            } else {
                vec![
                    (
                        if mode == 5 { yes } else { incoming },
                        if mode == 6 { result } else { default },
                    ),
                    (no, original),
                ]
            },
        };
        if mode == 8 && position == 0 {
            *i.get_mut(copy).unwrap() = ManagedInsn::FieldPut {
                object: Some(this),
                field: FieldRef::new(0),
                value: z,
            };
        }
        if mode == 9 && position == 0 {
            *i.get_mut(copy).unwrap() = ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![],
            };
        }
        if mode == 10 && position == 0 {
            cfg.add_edge(
                blocks[0],
                join,
                EdgeKind::Exception("java.lang.Exception".into()),
            )
            .unwrap();
        }
        if mode == 13 && position == 0 {
            cfg.add_edge(arm, arm, EdgeKind::Unconditional).unwrap();
        }
        previous = join;
        arguments.push(result);
    }
    let chain = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(if mode == 14 { 0 } else { 1 }),
        args: vec![
            this,
            arguments[if mode == 15 { 1 } else { 0 }],
            arguments[1],
        ],
    });
    let chain_block = cfg.add_block(chain);
    cfg.add_edge(previous, chain_block, EdgeKind::Unconditional)
        .unwrap();
    let ret = i.insert(ManagedInsn::Return { value: None });
    let exit = cfg.add_block(ret);
    cfg.add_edge(chain_block, exit, EdgeKind::Unconditional)
        .unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, i, v),
        DeclarationBindings {
            receiver: Some(this),
            parameters: vec![Some(a), Some(b), Some(mask), None],
            names: vec!["a".into(), "b".into(), "mask".into(), "marker".into()],
        },
    )
}
#[test]
fn sequential_shared_zero_choices_keep_original_selected_phi_edges() {
    let (ir, b) = fixture(0);
    let plan = mask_choices::plan(
        &ConstructorInput {
            reference: MethodRef::new(0),
            ir: &ir,
            bindings: &b,
        },
        &R,
    )
    .unwrap();
    assert_eq!(plan.target, MethodRef::new(1));
    assert_eq!(
        plan.steps
            .iter()
            .filter(|step| matches!(step, backend_java::branching::Step::If { .. }))
            .count(),
        2
    );
    let mut edges = std::collections::HashSet::new();
    for step in &plan.steps {
        if let backend_java::branching::Step::If {
            then_steps,
            else_steps,
            ..
        } = step
        {
            for arm in [then_steps, else_steps] {
                match arm.last().unwrap() {
                    backend_java::branching::Step::Copies { edge, copies } => {
                        assert!(edges.insert(*edge));
                        assert_eq!(copies.len(), 1);
                    }
                    _ => panic!("missing simultaneous copy"),
                }
            }
        }
    }
    assert!(!plan.prefix_blocks.contains(&plan.chain_block));
    let (ir, b) = fixture(16);
    assert!(
        mask_choices::plan(
            &ConstructorInput {
                reference: MethodRef::new(0),
                ir: &ir,
                bindings: &b
            },
            &R
        )
        .is_ok()
    );
}
#[test]
fn wrong_mask_polarity_effects_missing_phi_prethrow_edges_receiver_and_target_deny() {
    for mode in 1..=15 {
        let (ir, b) = fixture(mode);
        assert!(
            mask_choices::plan(
                &ConstructorInput {
                    reference: MethodRef::new(0),
                    ir: &ir,
                    bindings: &b
                },
                &R
            )
            .is_err(),
            "accepted mode {mode}"
        );
    }
}
fn primary() -> (MethodIr<SsaForm>, DeclarationBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let this = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(0)));
    let a = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(1)));
    let b = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(1)));
    let ids = [
        i.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(2),
            args: vec![this],
        }),
        i.insert(ManagedInsn::FieldPut {
            object: Some(this),
            field: FieldRef::new(0),
            value: a,
        }),
        i.insert(ManagedInsn::FieldPut {
            object: Some(this),
            field: FieldRef::new(1),
            value: b,
        }),
        i.insert(ManagedInsn::Return { value: None }),
    ];
    let mut cfg = Cfg::new(ids[0]);
    let mut previous = cfg.entry();
    for id in &ids[1..] {
        let block = cfg.add_block(*id);
        cfg.add_edge(previous, block, EdgeKind::Unconditional)
            .unwrap();
        previous = block;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, i, v),
        DeclarationBindings {
            receiver: Some(this),
            parameters: vec![Some(a), Some(b)],
            names: vec!["a".into(), "b".into()],
        },
    )
}
fn family(
    secondary: &MethodIr<SsaForm>,
    bindings: &DeclarationBindings,
) -> Result<Vec<ConstructorPlan>, backend_java::Unsupported> {
    let (primary, b) = primary();
    plan_constructor_family(
        "Fixture",
        &[
            ConstructorInput {
                reference: MethodRef::new(0),
                ir: secondary,
                bindings,
            },
            ConstructorInput {
                reference: MethodRef::new(1),
                ir: &primary,
                bindings: &b,
            },
        ],
        &[
            BlankFinal {
                reference: FieldRef::new(0),
                writes: [(MethodRef::new(1), 1)].into(),
            },
            BlankFinal {
                reference: FieldRef::new(1),
                writes: [(MethodRef::new(1), 1)].into(),
            },
        ],
        &R,
    )
}
#[test]
fn complete_original_family_still_requires_primary_final_writes() {
    let (ir, b) = fixture(16);
    let plans = family(&ir, &b).unwrap();
    assert_eq!(plans.len(), 2);
    let (primary, p) = primary();
    let inputs = [
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &ir,
            bindings: &b,
        },
        ConstructorInput {
            reference: MethodRef::new(1),
            ir: &primary,
            bindings: &p,
        },
    ];
    assert!(plan_constructor_family("Fixture", &inputs[..1], &[], &R).is_err());
    assert!(
        plan_constructor_family(
            "Fixture",
            &inputs,
            &[BlankFinal {
                reference: FieldRef::new(0),
                writes: [(MethodRef::new(1), 2)].into()
            }],
            &R
        )
        .is_err()
    );
}
#[test]
fn emitted_mask_constructor_matches_original_jvm_null_branch_bytecode() {
    let (ir, b) = fixture(16);
    let plans = family(&ir, &b).unwrap();
    let dir = std::env::temp_dir().join(format!("original-mask-choices-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "class Marker{{}} public class Fixture{{ final Object first,second;{} {} }}",
        plans[0].source, plans[1].source
    );
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    // ASM oracle retains NEW then the exact two original mask-bit branches,
    // local argument substitutions, and invokespecial of the primary overload.
    let oracle = r#"import jdk.internal.org.objectweb.asm.*;import java.nio.file.*;import java.lang.reflect.*;
public class MaskOracle implements Opcodes {
static void generate()throws Exception{ClassWriter c=new ClassWriter(0);c.visit(V17,ACC_PUBLIC,"Original",null,"java/lang/Object",null);c.visitField(ACC_PUBLIC|ACC_FINAL,"first","Ljava/lang/Object;",null,null).visitEnd();c.visitField(ACC_PUBLIC|ACC_FINAL,"second","Ljava/lang/Object;",null,null).visitEnd();MethodVisitor m=c.visitMethod(ACC_PUBLIC,"<init>","(Ljava/lang/Object;Ljava/lang/Object;)V",null,null);m.visitCode();m.visitVarInsn(ALOAD,0);m.visitMethodInsn(INVOKESPECIAL,"java/lang/Object","<init>","()V",false);for(int n=1;n<=2;n++){m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,n);m.visitFieldInsn(PUTFIELD,"Original",n==1?"first":"second","Ljava/lang/Object;");}m.visitInsn(RETURN);m.visitMaxs(2,3);m.visitEnd();m=c.visitMethod(ACC_PUBLIC|ACC_SYNTHETIC,"<init>","(Ljava/lang/Object;Ljava/lang/Object;ILMarker;)V",null,null);m.visitCode();for(int n=1;n<=2;n++){m.visitVarInsn(ILOAD,3);m.visitInsn(n==1?ICONST_1:ICONST_2);m.visitInsn(IAND);Label skip=new Label();m.visitJumpInsn(IFEQ,skip);m.visitInsn(ACONST_NULL);m.visitVarInsn(ASTORE,n);m.visitLabel(skip);m.visitFrame(F_SAME,0,null,0,null);}m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,1);m.visitVarInsn(ALOAD,2);m.visitMethodInsn(INVOKESPECIAL,"Original","<init>","(Ljava/lang/Object;Ljava/lang/Object;)V",false);m.visitInsn(RETURN);m.visitMaxs(3,5);m.visitEnd();c.visitEnd();Files.write(Path.of("Original.class"),c.toByteArray());}
public static void main(String[] ignored)throws Exception{generate();Class<?> original=Class.forName("Original");Constructor<?> ctor=original.getConstructor(Object.class,Object.class,int.class,Marker.class);int cases=0;for(int mask:new int[]{0,1,2,3,4,7,-1,Integer.MIN_VALUE,Integer.MAX_VALUE})for(int nulls=0;nulls<4;nulls++){Object a=(nulls&1)==0?new Object():null,b=(nulls&2)==0?new Object():null;Fixture f=new Fixture(a,b,mask,null);Object o=ctor.newInstance(a,b,mask,null);if(f.first!=original.getField("first").get(o)||f.second!=original.getField("second").get(o))throw new AssertionError("selected identity "+mask);cases++;}System.out.println("matched "+cases+" original-bytecode selected identities");}}
"#;
    // JDK25 no longer ships its old internal ASM. Generate the original
    // bytecode using JDK17, then compile/run the source comparison on JDK25.
    let split = oracle.find("public static void main").unwrap();
    let generator = format!(
        "{}public static void main(String[] ignored)throws Exception{{generate();}}}}",
        &oracle[..split]
    )
    .replace("class MaskOracle", "class MaskGenerator");
    std::fs::write(dir.join("MaskGenerator.java"), generator).unwrap();
    let oracle = format!(
        "import java.lang.reflect.*;public class MaskOracle{{{}",
        &oracle[split..]
    )
    .replace("{generate();Class<?> original", "{Class<?> original");
    std::fs::write(dir.join("MaskOracle.java"), oracle).unwrap();
    let generator_java = std::path::Path::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin");
    for (tool, args) in [
        (
            "javac",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "MaskGenerator.java",
            ],
        ),
        (
            "java",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "-cp",
                ".",
                "MaskGenerator",
            ],
        ),
    ] {
        let result = std::process::Command::new(generator_java.join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let java = std::path::Path::new("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin");
    let result = std::process::Command::new(java.join("javac"))
        .args(["Fixture.java", "MaskOracle.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = std::process::Command::new(java.join("java"))
        .args(["-Xverify:all", "-cp", ".", "MaskOracle"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap().trim(),
        "matched 36 original-bytecode selected identities"
    );
}
