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
    ops::{CompareOp, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

struct Resolver {
    hierarchy: bool,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        match r.index() {
            0 => Some("Fixture.Base".into()),
            1 => Some("Fixture.Left".into()),
            2 => Some("Fixture.Right".into()),
            _ => None,
        }
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (owner, name, is_static, parameters, return_type) = match r.index() {
            0 => ("Fixture.Base", "getRight", false, vec![], JavaType::Int),
            1 => (
                "Fixture",
                "observe",
                true,
                vec![JavaType::Boolean],
                JavaType::Void,
            ),
            2 => (
                "Fixture",
                "leftOnly",
                true,
                vec![JavaType::Class("Fixture.Left".into())],
                JavaType::Void,
            ),
            3 => (
                "Fixture",
                "bit",
                true,
                vec![JavaType::Boolean],
                JavaType::Boolean,
            ),
            _ => return None,
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            is_static,
            parameters,
            return_type,
            throws: vec![],
        })
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        match r.index() {
            0 => Some(1),
            1..=3 => Some(9),
            _ => None,
        }
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || self.hierarchy
                && *expected == JavaType::Class("Fixture.Base".into())
                && matches!(actual, JavaType::Class(n) if n == "Fixture.Left" || n == "Fixture.Right")
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
        let id = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(id),
            insns,
            values: ValueArena::with_key(),
        }
    }
    fn block(&mut self) -> BlockId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        self.cfg.add_block(id)
    }
    fn value(&mut self, block: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(block).unwrap().payload(), ty))
    }
    fn set(&mut self, block: BlockId, insn: ManagedInsn) {
        self.insns[*self.cfg.block(block).unwrap().payload()] = insn;
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
    fn edge(&mut self, a: BlockId, b: BlockId, kind: EdgeKind) -> decomp_core::ids::EdgeId {
        self.cfg.add_edge(a, b, kind).unwrap()
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> decomp_core::ids::EdgeId {
        self.edge(a, b, EdgeKind::Unconditional)
    }
}
// Mirrors TabLayout.C0's TextView/ImageView Phi -> != 0 -> View.getRight:
// the common receiver type is absent from every nominal SSA value annotation.
fn fixture(negative: u8) -> (MethodIr<SsaForm>, ParameterBindings, ValueId) {
    let mut b = Builder::new();
    let choose = b.incoming(ManagedType::Boolean);
    let left = b.incoming(if negative == 4 {
        ManagedType::Unknown
    } else {
        ManagedType::Reference(ClassRef::new(1))
    });
    let right = b.incoming(ManagedType::Reference(ClassRef::new(2)));
    let entry = b.cfg.entry();
    let l = b.block();
    let r = b.block();
    let yes = b.edge(entry, l, EdgeKind::BranchTrue);
    let no = b.edge(entry, r, EdgeKind::BranchFalse);
    b.set(
        entry,
        ManagedInsn::Branch {
            condition: Some(choose),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let a = b.value(
        l,
        if negative == 4 {
            ManagedType::Unknown
        } else {
            ManagedType::Reference(ClassRef::new(1))
        },
    );
    let c = b.value(r, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        l,
        ManagedInsn::Move {
            result: a,
            value: left,
        },
    );
    b.set(
        r,
        ManagedInsn::Move {
            result: c,
            value: right,
        },
    );
    let join = b.block();
    let phi = b.value(join, ManagedType::Unknown);
    let le = b.link(l, join);
    let re = b.link(r, join);
    b.set(
        join,
        ManagedInsn::Phi {
            result: phi,
            inputs: vec![(le, a), (re, c)],
        },
    );
    let zero_block = b.block();
    let zero = b.value(zero_block, ManagedType::Zero);
    b.set(
        zero_block,
        ManagedInsn::Const {
            result: zero,
            value: Constant::Zero,
        },
    );
    b.link(join, zero_block);
    let compare = b.block();
    let condition = b.value(compare, ManagedType::Boolean);
    b.set(
        compare,
        ManagedInsn::Compare {
            op: if negative == 3 {
                CompareOp::LessThan
            } else {
                CompareOp::NotEqual
            },
            result: condition,
            lhs: phi,
            rhs: zero,
        },
    );
    b.link(zero_block, compare);
    let observe = b.block();
    b.set(
        observe,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![condition],
        },
    );
    b.link(compare, observe);
    let invoke = b.block();
    let result = b.value(invoke, ManagedType::Int);
    b.set(
        invoke,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: Some(result),
            method: MethodRef::new(0),
            args: vec![phi],
        },
    );
    b.link(observe, invoke);
    let end = b.block();
    b.link(invoke, end);
    b.set(
        end,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    if negative == 1 {
        b.insns.insert(ManagedInsn::Opaque {
            family_tag: 3,
            reads: vec![phi],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if negative == 2 {
        b.insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![phi],
        });
    }
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        ParameterBindings {
            parameters: vec![choose, left, right],
            ..Default::default()
        },
        phi,
    )
}
fn method() -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: "select".into(),
        is_static: true,
        parameters: vec![
            JavaType::Boolean,
            JavaType::Class("Fixture.Left".into()),
            JavaType::Class("Fixture.Right".into()),
        ],
        return_type: JavaType::Int,
        throws: vec![],
    }
}

#[test]
fn common_receiver_candidate_survives_earlier_null_comparison() {
    let (ir, _, phi) = fixture(0);
    let r = Resolver { hierarchy: true };
    let nominal_types: Vec<_> = ir
        .values()
        .iter()
        .filter_map(|(_, v)| nominal(v.ty(), &r))
        .collect();
    assert!(!nominal_types.contains(&JavaType::Class("Fixture.Base".into())));
    assert!(consumer_constraints(&ir, &[phi], &JavaType::Int, &r).is_err());
    let declarations = declaration_plan(&ir, &JavaType::Int, &r).unwrap();
    assert_eq!(
        declarations.get(&phi),
        Some(&JavaType::Class("Fixture.Base".into()))
    );
    let plan = checked_phi_plan(
        &ir,
        phi,
        &JavaType::Class("Fixture.Base".into()),
        &JavaType::Int,
        &Hierarchy::default(),
        &r,
    )
    .unwrap();
    assert_eq!(plan.inputs.len(), 2);
    assert!(
        plan.inputs
            .iter()
            .all(|(_, coercion)| *coercion == Coercion::Identity)
    );
}
#[test]
fn candidate_discovery_never_discards_failed_consumer_or_unknown_producer_proof() {
    for negative in 1..=4 {
        let (ir, _, phi) = fixture(negative);
        let declarations =
            declaration_plan(&ir, &JavaType::Int, &Resolver { hierarchy: true }).unwrap();
        assert!(!declarations.contains_key(&phi), "negative={negative}");
    }
    let (ir, _, phi) = fixture(0);
    assert!(
        !declaration_plan(&ir, &JavaType::Int, &Resolver { hierarchy: false })
            .unwrap()
            .contains_key(&phi)
    );
}
#[test]
fn unchanged_emitted_common_receiver_method_compiles_and_preserves_selection_order_and_null() {
    let (ir, bindings, _) = fixture(0);
    let SourceResult::Complete { source, .. } =
        crate::emit_method(&ir, &method(), &bindings, &Resolver { hierarchy: true })
    else {
        panic!("bounded original shape must emit")
    };
    let dir =
        std::env::temp_dir().join(format!("decomp-contextual-consumer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let text = format!(
        r#"public class Fixture {{
static String trace="";
public static class Base {{ public int getRight() {{throw new AssertionError();}} }}
public static class Left extends Base {{ public int getRight() {{trace+="L";return 11;}} }}
public static class Right extends Base {{ public int getRight() {{trace+="R";return 29;}} }}
public static void observe(boolean present) {{trace+=present?"1":"0";}}
public static void leftOnly(Left value) {{}}
{source}
public static void main(String[] args) {{
Left l=new Left(); Right r=new Right();
if(select(true,l,r)!=11 || !trace.equals("1L")) throw new AssertionError(trace);
trace=""; if(select(false,l,r)!=29 || !trace.equals("1R")) throw new AssertionError(trace);
trace=""; try {{select(true,null,r);throw new AssertionError();}} catch(NullPointerException expected) {{}}
if(!trace.equals("0")) throw new AssertionError(trace);
trace=""; try {{select(false,l,null);throw new AssertionError();}} catch(NullPointerException expected) {{}}
if(!trace.equals("0")) throw new AssertionError(trace);
}} }}"#
    );
    std::fs::write(dir.join("Fixture.java"), text).unwrap();
    let jdk = std::env::var_os("JAVA_HOME").map(std::path::PathBuf::from);
    let tool = |name: &str| {
        jdk.as_ref()
            .map(|p| p.join("bin").join(name))
            .unwrap_or_else(|| name.into())
    };
    let compile = std::process::Command::new(tool("javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(tool("java"))
        .args(["-cp", ".", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn integer_domain_fixture(
    boolean_escape: bool,
    return_comparison: bool,
) -> (MethodIr<SsaForm>, ParameterBindings, ValueId, Vec<ValueId>) {
    let mut b = Builder::new();
    let choose = b.incoming(ManagedType::Boolean);
    let entry = b.cfg.entry();
    let a = b.block();
    let c = b.block();
    let yes = b.edge(entry, a, EdgeKind::BranchTrue);
    let no = b.edge(entry, c, EdgeKind::BranchFalse);
    b.set(
        entry,
        ManagedInsn::Branch {
            condition: Some(choose),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let first = b.value(a, ManagedType::Boolean);
    let second = b.value(c, ManagedType::Boolean);
    b.set(
        a,
        ManagedInsn::Const {
            result: first,
            value: Constant::Int(2),
        },
    );
    b.set(
        c,
        ManagedInsn::Const {
            result: second,
            value: Constant::Int(-1),
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
            inputs: vec![(ae, first), (ce, second)],
        },
    );
    let bit = b.block();
    let bool_value = b.value(bit, ManagedType::Boolean);
    b.set(
        bit,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(bool_value),
            method: MethodRef::new(3),
            args: vec![choose],
        },
    );
    b.link(join, bit);
    let cmp = b.block();
    let condition = b.value(cmp, ManagedType::Boolean);
    b.set(
        cmp,
        ManagedInsn::Compare {
            op: CompareOp::NotEqual,
            result: condition,
            lhs: phi,
            rhs: bool_value,
        },
    );
    b.link(bit, cmp);
    let observe = b.block();
    b.set(
        observe,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![condition],
        },
    );
    b.link(cmp, observe);
    let ret = b.block();
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(if return_comparison { condition } else { phi }),
        },
    );
    b.link(observe, ret);
    if boolean_escape {
        b.insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![first],
        });
    }
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        ParameterBindings {
            parameters: vec![choose],
            ..Default::default()
        },
        phi,
        vec![first, second],
    )
}
#[test]
fn non_boolean_integer_literals_and_phi_preserve_exact_bits_and_boolean_peer() {
    let (ir, bindings, phi, literals) = integer_domain_fixture(false, false);
    let r = Resolver { hierarchy: true };
    let declarations = declaration_plan(&ir, &JavaType::Int, &r).unwrap();
    for v in literals.iter().chain(std::iter::once(&phi)) {
        assert_eq!(declarations.get(v), Some(&JavaType::Int));
    }
    let signature = JavaMethod {
        owner: "Fixture".into(),
        name: "tri".into(),
        is_static: true,
        parameters: vec![JavaType::Boolean],
        return_type: JavaType::Int,
        throws: vec![],
    };
    let result = crate::emit_method(&ir, &signature, &bindings, &r);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let dir = std::env::temp_dir().join(format!(
        "decomp-contextual-integer-domain-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let text = format!(
        r#"public class Fixture {{
static String trace="";
public static boolean bit(boolean value) {{trace+="B";return value;}}
public static void observe(boolean value) {{trace+=value?"1":"0";}}
{source}
public static void main(String[] args) {{
if(tri(true)!=2 || !trace.equals("B1")) throw new AssertionError(trace);
trace="";if(tri(false)!=-1 || !trace.equals("B1")) throw new AssertionError(trace);
}} }}"#
    );
    std::fs::write(dir.join("Fixture.java"), text).unwrap();
    let home = std::env::var_os("JAVA_HOME").map(std::path::PathBuf::from);
    let tool = |name: &str| {
        home.as_ref()
            .map(|p| p.join("bin").join(name))
            .unwrap_or_else(|| name.into())
    };
    let compiled = std::process::Command::new(tool("javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = std::process::Command::new(tool("java"))
        .args(["-cp", ".", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
    let (ir, _, _, literals) = integer_domain_fixture(true, false);
    let declarations = declaration_plan(&ir, &JavaType::Int, &r).unwrap();
    assert!(
        !declarations.contains_key(&literals[0]),
        "a non-0/1 literal must never become a Boolean argument"
    );
}

#[test]
fn integer_candidate_can_come_from_exact_literal_domain_without_numeric_consumer() {
    let (ir, bindings, phi, _) = integer_domain_fixture(false, true);
    let r = Resolver { hierarchy: true };
    assert!(
        ir.values()
            .values()
            .all(|v| *v.ty() == ManagedType::Boolean)
    );
    let declarations = declaration_plan(&ir, &JavaType::Boolean, &r).unwrap();
    assert_eq!(declarations.get(&phi), Some(&JavaType::Int));
    let signature = JavaMethod {
        owner: "Fixture".into(),
        name: "compare".into(),
        is_static: true,
        parameters: vec![JavaType::Boolean],
        return_type: JavaType::Boolean,
        throws: vec![],
    };
    assert!(matches!(
        crate::emit_method(&ir, &signature, &bindings, &r),
        SourceResult::Complete { .. }
    ));
}
