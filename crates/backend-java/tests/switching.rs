//! Path-included switch helper tests, including actual JDK execution.
#![allow(clippy::unwrap_used)]
use backend_java::switching;

use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use decomp_structure::{ExtendedRegion, StructureError, structure_extended};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use switching::{ArmSource, SwitchError};

struct Fixture {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
}
impl Fixture {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let id = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(id),
            insns,
            values: ValueArena::with_key(),
        }
    }
    fn parameter(&mut self) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        self.values.insert(Value::new(id, ManagedType::Int))
    }
    fn block(&mut self) -> BlockId {
        self.cfg
            .add_block(self.insns.insert(ManagedInsn::Return { value: None }))
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        self.insns[*self.cfg.block(b).unwrap().payload()] = i;
    }
    fn edge(&mut self, a: BlockId, b: BlockId, kind: EdgeKind) -> EdgeId {
        self.cfg.add_edge(a, b, kind).unwrap()
    }
    fn value(&mut self, b: BlockId) -> ValueId {
        self.values.insert(Value::new(
            *self.cfg.block(b).unwrap().payload(),
            ManagedType::Int,
        ))
    }
    fn finish(self) -> MethodIr<SsaForm> {
        MethodIr::<SsaForm>::from_parts(self.cfg, self.insns, self.values)
    }
}
fn switch(root: &ExtendedRegion) -> &ExtendedRegion {
    match root {
        ExtendedRegion::Switch { .. } => root,
        ExtendedRegion::Sequence(rs) => rs
            .iter()
            .find(|r| matches!(r, ExtendedRegion::Switch { .. }))
            .unwrap(),
        _ => panic!("switch missing"),
    }
}
fn phi_fixture() -> (Fixture, Vec<ValueId>, Vec<ValueId>) {
    let mut f = Fixture::new();
    let bindings: Vec<_> = (0..4).map(|_| f.parameter()).collect();
    let h = f.cfg.entry();
    let first = f.block();
    let second = f.block();
    let tail = f.block();
    // Deliberately permuted adjacency, signed negative case and shared target.
    let default = f.edge(h, first, EdgeKind::Unconditional);
    let negative = f.edge(h, first, EdgeKind::Switch((-7i64) as u64));
    let positive = f.edge(h, first, EdgeKind::Switch(12));
    f.set(
        h,
        ManagedInsn::Switch {
            key: bindings[0],
            default_edge: default,
            cases: vec![(-7, negative), (12, positive)],
        },
    );
    let a = f.value(first);
    let b = f.value(second);
    let inputs = vec![
        (default, bindings[3]),
        (negative, bindings[1]),
        (positive, bindings[2]),
    ];
    f.set(
        first,
        ManagedInsn::Phi {
            result: a,
            inputs: inputs.clone(),
        },
    );
    f.set(
        second,
        ManagedInsn::Phi {
            result: b,
            inputs: vec![
                (default, bindings[1]),
                (negative, bindings[2]),
                (positive, bindings[3]),
            ],
        },
    );
    f.edge(first, second, EdgeKind::Unconditional);
    f.edge(second, tail, EdgeKind::Unconditional);
    f.set(tail, ManagedInsn::Return { value: Some(a) });
    (f, bindings, vec![a, b])
}

struct ProductionResolver;
impl backend_java::JavaResolver for ProductionResolver {
    fn method(&self, r: managed_ir::refs::MethodRef) -> Option<backend_java::JavaMethod> {
        Some(backend_java::JavaMethod {
            owner: "SwitchFixture".into(),
            name: format!("effect{}", r.index()),
            is_static: true,
            return_type: backend_java::JavaType::Int,
            parameters: vec![backend_java::JavaType::Int],
            throws: vec![],
        })
    }
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, _: managed_ir::refs::ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
}

fn production_source(ir: &MethodIr<SsaForm>, bindings: Vec<ValueId>) -> String {
    let method = backend_java::JavaMethod {
        owner: "SwitchFixture".into(),
        name: "choose".into(),
        is_static: true,
        return_type: backend_java::JavaType::Int,
        parameters: vec![backend_java::JavaType::Int; bindings.len()],
        throws: vec![],
    };
    match backend_java::emit_method(
        ir,
        &method,
        &backend_java::ParameterBindings {
            receiver: None,
            parameters: bindings,
            names: vec![],
        },
        &ProductionResolver,
    ) {
        backend_java::SourceResult::Complete { source, .. } => source,
        other => panic!("production lowering failed: {other:?}"),
    }
}

#[test]
fn production_grouped_default_entry_phis_compile_and_execute() {
    let (f, bindings, _) = phi_fixture();
    let source = production_source(&f.finish(), bindings);
    assert_eq!(source.matches("switch (").count(), 2);
    javac_run(
        &format!(
            "public class SwitchFixture {{ {source} public static void main(String[] a) {{ for(int k:new int[]{{-7,12,0,Integer.MIN_VALUE}}) System.out.println(choose(k,3,5,9)); }} }}"
        ),
        "3\n5\n9\n9\n",
    );
}

#[test]
fn production_selector_effect_and_continuation_phi_execute_once() {
    let mut f = Fixture::new();
    let parameter = f.parameter();
    let entry = f.cfg.entry();
    let key = f.value(entry);
    f.set(
        entry,
        ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Static,
            result: Some(key),
            method: managed_ir::refs::MethodRef::new(0),
            args: vec![parameter],
        },
    );
    let h = f.block();
    f.edge(entry, h, EdgeKind::Unconditional);
    let a = f.block();
    let b = f.block();
    let join = f.block();
    let tail = f.block();
    let ea = f.edge(h, a, EdgeKind::Switch((-7i64) as u64));
    let eb = f.edge(h, a, EdgeKind::Switch(12));
    let default = f.edge(h, b, EdgeKind::Unconditional);
    f.set(
        h,
        ManagedInsn::Switch {
            key,
            default_edge: default,
            cases: vec![(-7, ea), (12, eb)],
        },
    );
    let av = f.value(a);
    let bv = f.value(b);
    for (block, result, index) in [(a, av, 1), (b, bv, 2)] {
        f.set(
            block,
            ManagedInsn::Invoke {
                kind: managed_ir::ops::InvokeKind::Static,
                result: Some(result),
                method: managed_ir::refs::MethodRef::new(index),
                args: vec![key],
            },
        );
    }
    let aj = f.edge(a, join, EdgeKind::Unconditional);
    let bj = f.edge(b, join, EdgeKind::Unconditional);
    let result = f.value(join);
    f.set(
        join,
        ManagedInsn::Phi {
            result,
            inputs: vec![(aj, av), (bj, bv)],
        },
    );
    f.edge(join, tail, EdgeKind::Unconditional);
    f.set(
        tail,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    let source = production_source(&f.finish(), vec![parameter]);
    assert_eq!(source.matches("SwitchFixture.effect0(").count(), 1);
    assert_eq!(source.matches("SwitchFixture.effect1(").count(), 1);
    javac_run(
        &format!(
            "public class SwitchFixture {{ static String trace=\"\"; static int effect0(int k) {{trace+=\"K\";return k;}} static int effect1(int k) {{trace+=\"A\";return k+1;}} static int effect2(int k) {{trace+=\"D\";return k+2;}} {source} public static void main(String[] a) {{for(int k:new int[]{{-7,12,3}}) {{trace=\"\";System.out.println(choose(k)+\":\"+trace);}}}} }}"
        ),
        "-6:KA\n13:KA\n5:KD\n",
    );
}

fn javac_run(source: &str, expected: &str) {
    let dir = std::env::temp_dir().join(format!(
        "decomphify-switch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("SwitchFixture.java"), source).unwrap();
    let bin = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let compile = std::process::Command::new(format!("{bin}/javac"))
        .arg("SwitchFixture.java")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(format!("{bin}/java"))
        .args(["-cp", ".", "SwitchFixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8(run.stdout).unwrap(), expected);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn negative_default_shared_cases_and_multi_phi_compile_and_execute() {
    let (f, bindings, results) = phi_fixture();
    let ir = f.finish();
    let root = structure_extended(ir.cfg()).unwrap().root;
    let p = switching::plan(&ir, switch(&root), |v| bindings.contains(&v)).unwrap();
    assert_eq!(p.arms.len(), 1);
    assert!(p.dispatch.iter().all(|d| d.copies.len() == 2));
    let mut n = 0;
    let body = switching::emit::<std::convert::Infallible>(
        &p,
        |v| {
            Ok(if let Some(i) = bindings.iter().position(|x| *x == v) {
                format!("p{i}")
            } else {
                format!("v{}", results.iter().position(|x| *x == v).unwrap())
            })
        },
        |_| Ok("int".into()),
        || {
            n += 1;
            format!("edge{n}")
        },
        |_| {
            Ok(ArmSource {
                source: String::new(),
                continues: true,
            })
        },
    )
    .unwrap();
    for entry in body.split("break;") {
        if let (Some(snapshot), Some(assign)) = (entry.rfind("int edge"), entry.find("v0 =")) {
            assert!(snapshot < assign);
        }
    }
    javac_run(
        &format!(
            "public class SwitchFixture {{ static int choose(int p0,int p1,int p2,int p3) {{ int v0; int v1; {body} return v0*100+v1; }} public static void main(String[] a) {{ for(int k:new int[]{{-7,12,0,Integer.MIN_VALUE}}) System.out.println(choose(k,3,5,9)); }} }}"
        ),
        "305\n509\n903\n903\n",
    );
}

#[test]
fn grouped_effects_terminate_without_unreachable_break_or_fallthrough() {
    let mut f = Fixture::new();
    let key = f.parameter();
    let h = f.cfg.entry();
    let a = f.block();
    let b = f.block();
    let ea = f.edge(h, a, EdgeKind::Switch((-1i64) as u64));
    let eb = f.edge(h, a, EdgeKind::Switch(4));
    let default = f.edge(h, b, EdgeKind::Unconditional);
    f.set(
        h,
        ManagedInsn::Switch {
            key,
            default_edge: default,
            cases: vec![(-1, ea), (4, eb)],
        },
    );
    let at = f.block();
    let bt = f.block();
    f.set(
        a,
        ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Static,
            result: None,
            method: managed_ir::refs::MethodRef::new(1),
            args: vec![],
        },
    );
    f.set(
        b,
        ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Static,
            result: None,
            method: managed_ir::refs::MethodRef::new(2),
            args: vec![],
        },
    );
    f.edge(a, at, EdgeKind::Unconditional);
    f.edge(b, bt, EdgeKind::Unconditional);
    f.set(at, ManagedInsn::Return { value: Some(key) });
    f.set(bt, ManagedInsn::Return { value: Some(key) });
    let ir = f.finish();
    let root = structure_extended(ir.cfg()).unwrap().root;
    let p = switching::plan(&ir, &root, |_| true).unwrap();
    let body = switching::emit::<std::convert::Infallible>(
        &p,
        |_| Ok("key".into()),
        |_| Ok("int".into()),
        || "unused".into(),
        |region| {
            let ExtendedRegion::Sequence(blocks) = region else {
                panic!("unexpected region")
            };
            let mut source = String::new();
            for region in blocks {
                let ExtendedRegion::Block(block) = region else {
                    panic!("unexpected child")
                };
                match ir
                    .instruction(*ir.cfg().block(*block).unwrap().payload())
                    .unwrap()
                {
                    ManagedInsn::Invoke { method, .. } => source.push_str(if method.index() == 1 {
                        "trace += \"A\";"
                    } else {
                        "trace += \"D\";"
                    }),
                    ManagedInsn::Return { .. } => source.push_str("return key;"),
                    other => panic!("unexpected instruction {other:?}"),
                }
            }
            Ok(ArmSource {
                source,
                continues: false,
            })
        },
    )
    .unwrap();
    assert_eq!(body.matches("trace += \"A\"").count(), 1);
    assert!(!body.contains("break;"));
    javac_run(
        &format!(
            "public class SwitchFixture {{ static String trace=\"\"; static int choose(int key) {{ trace += \"K\"; {body} }} public static void main(String[] a) {{ for(int k:new int[]{{-1,4,0}}) {{ trace=\"\"; int v=choose(k); System.out.println(v+\":\"+trace); }} }} }}"
        ),
        "-1:KA\n4:KA\n0:KD\n",
    );
}

#[test]
fn malformed_edges_phi_types_and_availability_are_typed() {
    for mode in 0..6 {
        let (mut f, bindings, _) = phi_fixture();
        let h = f.cfg.entry();
        let id = *f.cfg.block(h).unwrap().payload();
        if mode == 0 {
            if let ManagedInsn::Switch { cases, .. } = &mut f.insns[id] {
                cases[0].0 = 99;
            }
        } else if mode == 1 {
            if let ManagedInsn::Switch {
                cases,
                default_edge,
                ..
            } = &mut f.insns[id]
            {
                *default_edge = cases[0].1;
            }
        } else if mode == 2 {
            let phi = f
                .insns
                .iter()
                .find_map(|(id, i)| matches!(i, ManagedInsn::Phi { .. }).then_some(id))
                .unwrap();
            if let ManagedInsn::Phi { inputs, .. } = &mut f.insns[phi] {
                inputs.pop();
            }
        } else if mode == 3
            && let ManagedInsn::Switch { cases, .. } = &mut f.insns[id]
        {
            cases[0].0 = i64::MAX;
        }
        let ir = f.finish();
        let root = structure_extended(ir.cfg()).unwrap().root;
        let error = switching::plan(&ir, switch(&root), |v| {
            mode != 4 && (mode != 5 || v == bindings[0])
        })
        .unwrap_err();
        match mode {
            0 | 1 | 3 => assert!(matches!(error, SwitchError::InvalidEdges(_))),
            2 => assert!(matches!(error, SwitchError::InvalidPhi(_))),
            4 | 5 => assert!(matches!(error, SwitchError::Unavailable(_))),
            _ => unreachable!(),
        }
    }
}

#[test]
fn exception_edges_keep_structure_error() {
    let (mut f, bindings, _) = phi_fixture();
    let b = f.block();
    f.edge(f.cfg.entry(), b, EdgeKind::Exception("E".into()));
    let ir = f.finish();
    assert!(matches!(
        switching::plan(&ir, &ExtendedRegion::Block(ir.cfg().entry()), |v| bindings
            .contains(&v)),
        Err(SwitchError::Structure(
            StructureError::UnsupportedEdge { .. }
        ))
    ));
}

#[test]
fn selector_type_and_phi_source_type_are_checked() {
    for selector in [true, false] {
        let (mut f, bindings, _) = phi_fixture();
        let value = bindings[usize::from(!selector)];
        let def = f.values[value].def();
        f.values[value] = Value::new(def, ManagedType::Long);
        let ir = f.finish();
        let root = structure_extended(ir.cfg()).unwrap().root;
        assert!(matches!(
            switching::plan(&ir, switch(&root), |_| true),
            Err(SwitchError::TypeMismatch(_))
        ));
    }
}

#[test]
fn continuing_arms_break_before_join_effect_and_selector_evaluates_once() {
    let mut f = Fixture::new();
    let key = f.parameter();
    let h = f.cfg.entry();
    let a = f.block();
    let b = f.block();
    let join = f.block();
    let ea = f.edge(h, a, EdgeKind::Switch(1));
    let default = f.edge(h, b, EdgeKind::Unconditional);
    f.set(
        h,
        ManagedInsn::Switch {
            key,
            default_edge: default,
            cases: vec![(1, ea)],
        },
    );
    for (block, index) in [(a, 1), (b, 2)] {
        f.set(
            block,
            ManagedInsn::Invoke {
                kind: managed_ir::ops::InvokeKind::Static,
                result: None,
                method: managed_ir::refs::MethodRef::new(index),
                args: vec![],
            },
        );
        f.edge(block, join, EdgeKind::Unconditional);
    }
    f.set(join, ManagedInsn::Return { value: Some(key) });
    let ir = f.finish();
    let root = structure_extended(ir.cfg()).unwrap().root;
    let p = switching::plan(&ir, switch(&root), |_| true).unwrap();
    let body = switching::emit::<std::convert::Infallible>(
        &p,
        |_| Ok("key".into()),
        |_| Ok("int".into()),
        || "unused".into(),
        |region| {
            let ExtendedRegion::Block(block) = region else {
                panic!("unexpected region")
            };
            assert_ne!(*block, join);
            Ok(ArmSource {
                source: format!("trace += \"{}\";", if *block == a { "A" } else { "D" }),
                continues: true,
            })
        },
    )
    .unwrap();
    assert_eq!(body.matches("break;").count(), 2);
    javac_run(
        &format!(
            "public class SwitchFixture {{ static String trace=\"\"; static int selector(int k) {{trace+=\"K\";return k;}} static int choose(int k) {{ int key=selector(k); {body} trace+=\"J\";return key; }} public static void main(String[] a) {{for(int k:new int[]{{1,8}}) {{ trace=\"\";System.out.println(choose(k)+\":\"+trace); }} }} }}"
        ),
        "1:KAJ\n8:KDJ\n",
    );
}

#[test]
fn overlapping_fallthrough_is_rejected_instead_of_duplicating_effects() {
    let mut f = Fixture::new();
    let key = f.parameter();
    let h = f.cfg.entry();
    let a = f.block();
    let b = f.block();
    let d = f.block();
    let ea = f.edge(h, a, EdgeKind::Switch(1));
    let eb = f.edge(h, b, EdgeKind::Switch(2));
    let default = f.edge(h, d, EdgeKind::Unconditional);
    f.set(
        h,
        ManagedInsn::Switch {
            key,
            default_edge: default,
            cases: vec![(1, ea), (2, eb)],
        },
    );
    let ab = f.edge(a, b, EdgeKind::Unconditional);
    f.set(
        a,
        ManagedInsn::Branch {
            condition: None,
            then_edge: ab,
            else_edge: None,
        },
    );
    let ir = f.finish();
    assert!(matches!(
        switching::plan(&ir, &ExtendedRegion::Block(h), |_| true),
        Err(SwitchError::Structure(StructureError::OverlappingRegion(_)))
    ));
}
