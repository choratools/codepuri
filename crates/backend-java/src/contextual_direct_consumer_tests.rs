//! Whole-family bit typing must match the actual source consumer grammar.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaField, JavaMethod, ParameterBindings, SourceResult};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::BlockId,
};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture".into(),
            name: "observe".into(),
            is_static: true,
            parameters: vec![if r.index() == 0 {
                JavaType::Boolean
            } else {
                JavaType::Class("java.lang.Object".into())
            }],
            return_type: JavaType::Void,
            throws: vec![],
        })
    }
}
struct Builder {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
}
impl Builder {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let entry = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(entry),
            insns,
            values: ValueArena::with_key(),
        }
    }
    fn block(&mut self) -> BlockId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        self.cfg.add_block(id)
    }
    fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), ty))
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        self.insns[*self.cfg.block(b).unwrap().payload()] = i;
    }
    fn incoming(&mut self, ty: ManagedType) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        let v = self.values.insert(Value::new(id, ty));
        self.insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        };
        v
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> decomp_core::ids::EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
}
// A Boolean Phi -> nominal-int Move is one atomic family, as in original39055.
// kind: 0 array read, 1 array store, 2 allocation length, 3 switch, 4 branch.
fn fixture(kind: u8, negative: u8) -> (MethodIr<SsaForm>, ParameterBindings, ValueId, ValueId) {
    let mut b = Builder::new();
    let choose = b.incoming(ManagedType::Boolean);
    let array = b.incoming(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let entry = b.cfg.entry();
    let a = b.block();
    let c = b.block();
    let yes = b.cfg.add_edge(entry, a, EdgeKind::BranchTrue).unwrap();
    let no = b.cfg.add_edge(entry, c, EdgeKind::BranchFalse).unwrap();
    b.set(
        entry,
        ManagedInsn::Branch {
            condition: Some(choose),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let x = b.value(
        a,
        if negative == 3 {
            ManagedType::Unknown
        } else {
            ManagedType::Int
        },
    );
    let y = b.value(c, ManagedType::Int);
    b.set(
        a,
        if negative == 3 {
            ManagedInsn::Opaque {
                family_tag: 7,
                reads: vec![],
                writes: vec![x],
                effects: EffectFlags::NONE,
            }
        } else {
            ManagedInsn::Const {
                result: x,
                value: Constant::Int(if negative == 2 { 2 } else { 1 }),
            }
        },
    );
    b.set(
        c,
        ManagedInsn::Const {
            result: y,
            value: Constant::Int(0),
        },
    );
    let join = b.block();
    let phi = b.value(join, ManagedType::Boolean);
    let ae = b.link(a, join);
    let ce = b.link(c, join);
    b.set(
        join,
        ManagedInsn::Phi {
            result: phi,
            inputs: vec![(ae, x), (ce, y)],
        },
    );
    let copied = b.block();
    let copy = b.value(
        copied,
        if kind == 4 {
            ManagedType::Boolean
        } else {
            ManagedType::Int
        },
    );
    b.set(
        copied,
        ManagedInsn::Move {
            result: copy,
            value: phi,
        },
    );
    b.link(join, copied);
    if negative == 1 {
        let id = b.insns.insert(ManagedInsn::Return { value: None });
        b.insns[id] = ManagedInsn::Branch {
            condition: Some(copy),
            then_edge: ae,
            else_edge: Some(ce),
        };
    }
    if negative == 2 || negative == 4 {
        b.insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(u32::from(negative == 4)),
            args: vec![phi],
        });
    }
    let body = b.block();
    b.link(copied, body);
    if kind >= 3 {
        let t = b.block();
        let f = b.block();
        let te = b
            .cfg
            .add_edge(
                body,
                t,
                if kind == 3 {
                    EdgeKind::Switch(1)
                } else {
                    EdgeKind::BranchTrue
                },
            )
            .unwrap();
        let fe = b
            .cfg
            .add_edge(
                body,
                f,
                if kind == 3 {
                    EdgeKind::Unconditional
                } else {
                    EdgeKind::BranchFalse
                },
            )
            .unwrap();
        b.set(
            body,
            if kind == 3 {
                ManagedInsn::Switch {
                    key: copy,
                    default_edge: fe,
                    cases: vec![(1, te)],
                }
            } else {
                ManagedInsn::Branch {
                    condition: Some(copy),
                    then_edge: te,
                    else_edge: Some(fe),
                }
            },
        );
        for (block, n) in [(t, 9), (f, 12)] {
            let v = b.value(block, ManagedType::Int);
            b.set(
                block,
                ManagedInsn::Const {
                    result: v,
                    value: Constant::Int(n),
                },
            );
            let ret = b.block();
            b.link(block, ret);
            b.set(ret, ManagedInsn::Return { value: Some(v) });
        }
    } else {
        let result = if kind == 0 {
            let v = b.value(body, ManagedType::Int);
            b.set(
                body,
                ManagedInsn::ArrayGet {
                    result: v,
                    array,
                    index: copy,
                },
            );
            v
        } else if kind == 1 {
            b.set(
                body,
                ManagedInsn::ArrayPut {
                    array,
                    index: copy,
                    value: copy,
                },
            );
            copy
        } else {
            let allocated = b.value(
                body,
                ManagedType::Array {
                    element: Box::new(ManagedType::Int),
                    nullable: false,
                },
            );
            b.set(
                body,
                ManagedInsn::NewArray {
                    result: allocated,
                    element: managed_ir::refs::TypeRef::new(0),
                    length: copy,
                },
            );
            let length = b.block();
            b.link(body, length);
            let v = b.value(length, ManagedType::Int);
            b.set(
                length,
                ManagedInsn::ArrayLength {
                    result: v,
                    array: allocated,
                },
            );
            let ret = b.block();
            b.link(length, ret);
            b.set(ret, ManagedInsn::Return { value: Some(v) });
            return (
                MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
                ParameterBindings {
                    parameters: vec![choose, array],
                    ..Default::default()
                },
                phi,
                copy,
            );
        };
        let observe = b.block();
        b.link(body, observe);
        b.set(
            observe,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(0),
                args: vec![copy],
            },
        );
        let ret = b.block();
        b.link(observe, ret);
        b.set(
            ret,
            ManagedInsn::Return {
                value: Some(result),
            },
        );
    }
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        ParameterBindings {
            parameters: vec![choose, array],
            ..Default::default()
        },
        phi,
        copy,
    )
}
struct TypedResolver;
impl JavaResolver for TypedResolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Resolver.class(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        Resolver.field(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Resolver.string(r)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Resolver.method(r)
    }
    fn instruction_type(&self, _: managed_ir::refs::TypeRef) -> Option<JavaType> {
        Some(JavaType::Int)
    }
}
fn method(kind: u8) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: format!("test{kind}"),
        is_static: true,
        parameters: vec![JavaType::Boolean, JavaType::Array(Box::new(JavaType::Int))],
        return_type: JavaType::Int,
        throws: vec![],
    }
}
#[test]
fn direct_integer_consumers_select_int_for_whole_boolean_bit_family() {
    for kind in 0..=4 {
        let (ir, _, phi, copy) = fixture(kind, 0);
        let expected = if kind == 4 {
            JavaType::Boolean
        } else {
            JavaType::Int
        };
        let declarations = declaration_plan(&ir, &JavaType::Int, &TypedResolver).unwrap();
        assert_eq!(declarations.get(&phi), Some(&expected), "kind={kind}");
        assert_eq!(declarations.get(&copy), Some(&expected));
        let wrong = if kind == 4 {
            JavaType::Int
        } else {
            JavaType::Boolean
        };
        assert!(
            checked_phi_plan(
                &ir,
                phi,
                &wrong,
                &JavaType::Int,
                &Hierarchy::default(),
                &TypedResolver
            )
            .is_err()
        );
    }
}
#[test]
fn direct_family_typing_retains_incompatible_unknown_and_reference_denials() {
    for negative in 1..=4 {
        let (ir, bindings, phi, _) = fixture(0, negative);
        let declarations = declaration_plan(&ir, &JavaType::Int, &TypedResolver).unwrap();
        assert!(!declarations.contains_key(&phi), "negative={negative}");
        assert!(matches!(
            crate::emit_method(&ir, &method(0), &bindings, &TypedResolver),
            SourceResult::Unsupported(_)
        ));
    }
}
#[test]
fn unchanged_direct_consumer_sources_compile_and_keep_index_failure_order() {
    let mut source = String::new();
    for kind in 0..=4 {
        let (ir, b, _, _) = fixture(kind, 0);
        let emitted = crate::emit_method(&ir, &method(kind), &b, &TypedResolver);
        let SourceResult::Complete { source: s, .. } = emitted else {
            panic!("kind={kind} {emitted:?}")
        };
        source.push_str(&s);
    }
    let dir = std::env::temp_dir().join(format!(
        "contextual-direct-consumers-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class Fixture {{ static String trace="";static void observe(boolean v) {{trace+=v?"1":"0";}} {source}
    public static void main(String[] args) {{ int[] a={{11,29}}; if(test0(true,a)!=29||!trace.equals("1"))throw new AssertionError(trace);trace="";if(test0(false,a)!=11||!trace.equals("0"))throw new AssertionError(trace);trace="";try{{test0(true,new int[1]);throw new AssertionError();}}catch(ArrayIndexOutOfBoundsException expected){{}}if(!trace.isEmpty())throw new AssertionError(trace);try{{test0(false,null);throw new AssertionError();}}catch(NullPointerException expected){{}}if(!trace.isEmpty())throw new AssertionError(trace);if(test1(true,a)!=1||a[1]!=1)throw new AssertionError();if(test2(true,a)!=1||test2(false,a)!=0)throw new AssertionError();for(boolean bit:new boolean[]{{true,false}})if(test3(bit,a)!=(bit?9:12)||test4(bit,a)!=(bit?9:12))throw new AssertionError();}} }}"#
    );
    std::fs::write(dir.join("Fixture.java"), java).unwrap();
    let home = std::env::var_os("JAVA_HOME").map(std::path::PathBuf::from);
    let tool = |name: &str| {
        home.as_ref()
            .map(|p| p.join("bin").join(name))
            .unwrap_or_else(|| name.into())
    };
    let out = std::process::Command::new(tool("javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new(tool("java"))
        .args(["-cp", ".", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn only_attested_initial_boolean_parameters_extend_bit_evidence() {
    for tag in [0x8001, 123] {
        let mut b = Builder::new();
        let initial = b.incoming(ManagedType::Boolean);
        let def = b.values[initial].def();
        if let ManagedInsn::Opaque { family_tag, .. } = &mut b.insns[def] {
            *family_tag = tag;
        }
        let copied = b.block();
        let copy = b.value(copied, ManagedType::Boolean);
        b.set(
            copied,
            ManagedInsn::Move {
                result: copy,
                value: initial,
            },
        );
        let ir = MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values);
        let prove = |trusted: bool| {
            category_one_expression_with_parameters(
                &ir,
                copy,
                &JavaType::Boolean,
                &JavaType::Int,
                &Resolver,
                &|v| trusted && v == initial,
            )
        };
        assert_eq!(prove(false), None);
        assert_eq!(
            prove(true),
            if tag == 0x8001 {
                Some(CategoryOneCoercion::BooleanToInt)
            } else {
                None
            }
        );
    }
}

#[test]
fn signature_boolean_parameter_emits_exact_numeric_return() {
    let mut b = Builder::new();
    let initial = b.incoming(ManagedType::Boolean);
    let entry = b.cfg.entry();
    b.set(
        entry,
        ManagedInsn::Return {
            value: Some(initial),
        },
    );
    let ir = MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values);
    let declaration = JavaMethod {
        owner: "BitParameter".into(),
        name: "bit".into(),
        is_static: true,
        parameters: vec![JavaType::Boolean],
        return_type: JavaType::Int,
        throws: vec![],
    };
    let bindings = ParameterBindings {
        receiver: None,
        parameters: vec![initial],
        names: Default::default(),
    };
    let SourceResult::Complete { source, .. } =
        crate::emit_method(&ir, &declaration, &bindings, &Resolver)
    else {
        panic!("attested parameter must emit")
    };
    assert!(source.contains("? 1 : 0"), "{source}");
    let dir = std::env::temp_dir().join(format!("decomphify-bit-parameter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let array_source = boolean_literal_index_source();
    let java = format!(
        "public class BitParameter {{ {source} {array_source} public static void main(String[] args) {{ if(bit(false)!=0 || bit(true)!=1 || element(new int[]{{7,9}})!=9) throw new AssertionError(); try {{ element(null); throw new AssertionError(); }} catch(NullPointerException expected) {{}} try {{ element(new int[1]); throw new AssertionError(); }} catch(ArrayIndexOutOfBoundsException expected) {{}} }} }}"
    );
    std::fs::write(dir.join("BitParameter.java"), java).unwrap();
    let home = std::env::var_os("JAVA_HOME").map(std::path::PathBuf::from);
    for (tool, args) in [
        ("javac", vec!["--release", "17", "BitParameter.java"]),
        ("java", vec!["-cp", ".", "BitParameter"]),
    ] {
        let executable = home
            .as_ref()
            .map(|p| p.join("bin").join(tool))
            .unwrap_or_else(|| tool.into());
        let out = std::process::Command::new(executable)
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn boolean_literal_index_source() -> String {
    let mut b = Builder::new();
    let array = b.incoming(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let entry = b.cfg.entry();
    let index = b.value(entry, ManagedType::Boolean);
    b.set(
        entry,
        ManagedInsn::Const {
            result: index,
            value: Constant::Boolean(true),
        },
    );
    let load = b.block();
    b.link(entry, load);
    let result = b.value(load, ManagedType::Int);
    b.set(
        load,
        ManagedInsn::ArrayGet {
            result,
            array,
            index,
        },
    );
    let ret = b.block();
    b.link(load, ret);
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    let ir = MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values);
    let declaration = JavaMethod {
        owner: "BitParameter".into(),
        name: "element".into(),
        is_static: true,
        parameters: vec![JavaType::Array(Box::new(JavaType::Int))],
        return_type: JavaType::Int,
        throws: vec![],
    };
    let bindings = ParameterBindings {
        parameters: vec![array],
        ..Default::default()
    };
    let SourceResult::Complete { source, .. } =
        crate::emit_method(&ir, &declaration, &bindings, &Resolver)
    else {
        panic!("proven Boolean literal index must emit")
    };
    source
}
