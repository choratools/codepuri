//! Read-index cost, physical-word multiplicity, and allocation-effect regression.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};
use std::cell::RefCell;
#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, ty: ManagedType, f: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut value = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}

#[derive(Default)]
struct CountingResolver {
    calls: RefCell<HashMap<u32, usize>>,
    two: bool,
}
impl JavaResolver for CountingResolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture.Outer".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        *self.calls.borrow_mut().entry(r.index()).or_default() += 1;
        let (owner, name, is_static, parameters, return_type) = match r.index() {
            0 => (
                "Fixture.Outer",
                "<init>",
                false,
                vec![JavaType::Long; 8],
                JavaType::Void,
            ),
            1 => (
                "Fixture",
                "helper",
                true,
                vec![JavaType::Long; if self.two { 2 } else { 1 }],
                JavaType::Long,
            ),
            2 => (
                "Fixture",
                "pair",
                true,
                vec![JavaType::Long; 2],
                JavaType::Long,
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
}
fn constant(b: &mut Builder, n: i64) -> ValueId {
    b.value(ManagedType::Long, |result| ManagedInsn::Const {
        result,
        value: Constant::Long(n),
    })
}
#[test]
fn read_index_caches_relevant_signatures_preserves_multiplicity_and_raw_fallback() {
    let mut b = Builder::default();
    let v = constant(&mut b, 1);
    let other = constant(&mut b, 2);
    for (method, args) in [
        (1, vec![v, v]),
        (1, vec![v, v]),
        (2, vec![v, v]),
        (2, vec![v, v, v, v]),
        (1, vec![v, other]),
        (98, vec![v, v]),
        (98, vec![v, v]),
    ] {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(method),
            args,
        });
    }
    for _ in 0..400 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(99),
            args: vec![other],
        });
    }
    let ir = b.finish();
    let wanted = HashSet::from([v]);
    let r = CountingResolver::default();
    let index = read_index(&ir, &wanted, &r);
    assert_eq!(index[&v].len(), 11); // 1+1+2+2+1+2+2
    assert_eq!(*r.calls.borrow(), HashMap::from([(1, 1), (2, 1), (98, 1)]));
    let changed = CountingResolver {
        two: true,
        ..Default::default()
    };
    assert_eq!(read_index(&ir, &wanted, &changed)[&v].len(), 13);
    // A distinct physical high word remains visible on invalid metadata.
    let both = read_index(&ir, &HashSet::from([v, other]), &r);
    assert!(both[&other].len() >= 401);
}
fn fixture(noise: bool) -> (MethodIr<SsaForm>, InsnId) {
    let mut b = Builder::default();
    let outside = constant(&mut b, 99);
    let outer = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let allocation = *b.order.last().unwrap();
    let mut args = vec![outer];
    for n in 1..=8 {
        let v = constant(&mut b, n);
        let result = b.value(ManagedType::Long, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(1),
            args: vec![v, v],
        });
        args.extend([result, result]);
    }
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args,
    });
    b.push(ManagedInsn::Return { value: Some(outer) });
    if noise {
        for _ in 0..400 {
            b.insns.insert(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(99),
                args: vec![outside],
            });
        }
    }
    (b.finish(), allocation)
}
#[test]
fn read_index_allocation_plan_skips_foreign_invokes_and_preserves_runtime_trace() {
    let (ir, allocation) = fixture(true);
    let r = CountingResolver::default();
    let emitter = Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir: &ir,
        resolver: &r,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: None,
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
    };
    let plan = emitter.argument_object_plan(allocation).unwrap();
    assert_eq!(plan.argument_gap.len(), 16);
    assert_eq!(r.calls.borrow().get(&99), None);
    assert_eq!(r.calls.borrow().get(&1), Some(&9));
    let (ir, _) = fixture(false);
    let result = emit_method(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "construct".into(),
            is_static: true,
            return_type: JavaType::Class("Fixture.Outer".into()),
            parameters: vec![],
            throws: vec![],
        },
        &ParameterBindings::default(),
        &CountingResolver::default(),
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let java = format!(
        "public class Fixture {{ static String trace=\"\"; static long helper(long n){{trace+=n;return n;}} static class Outer {{ static {{trace+=\"A\";}} Outer(long a,long b,long c,long d,long e,long f,long g,long h){{if(a!=1||b!=2||c!=3||d!=4||e!=5||f!=6||g!=7||h!=8)throw new AssertionError();trace+=\"C\";}} }} {source} public static void main(String[]a){{construct();if(!trace.equals(\"A12345678C\"))throw new AssertionError(trace);}} }}"
    );
    let dir = std::env::temp_dir().join(format!("object-read-index-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), java).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = std::process::Command::new(format!("{home}/bin/{tool}"))
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
    std::fs::remove_dir_all(dir).unwrap();
}
