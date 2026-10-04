//! Static-final source legality, loop effects and original edge Phi batches.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::{cfg::Cfg, ids::BlockId};
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

struct Fixture {
    cfg: Cfg<InsnId>,
    instructions: InsnArena,
    values: ValueArena,
}
impl Fixture {
    fn new() -> Self {
        let mut instructions = InsnArena::default();
        let first = instructions.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(first),
            instructions,
            values: ValueArena::default(),
        }
    }
    fn block(&mut self) -> BlockId {
        self.cfg.add_block(
            self.instructions
                .insert(ManagedInsn::Return { value: None }),
        )
    }
    fn set(&mut self, block: BlockId, instruction: ManagedInsn) {
        self.instructions[*self.cfg.block(block).unwrap().payload()] = instruction;
    }
    fn value(&mut self, block: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(block).unwrap().payload(), ty))
    }
    fn link(&mut self, from: BlockId, to: BlockId) -> decomp_core::ids::EdgeId {
        self.cfg
            .add_edge(from, to, EdgeKind::Unconditional)
            .unwrap()
    }
    fn constant(&mut self, tail: BlockId, number: i32) -> (BlockId, ValueId) {
        let b = self.block();
        let result = self.value(b, ManagedType::Int);
        self.set(
            b,
            ManagedInsn::Const {
                result,
                value: Constant::Int(number),
            },
        );
        self.link(tail, b);
        (b, result)
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        let (name, return_type, parameters) = match reference.index() {
            0 => ("seed", JavaType::Int, vec![]),
            1 => ("test", JavaType::Boolean, vec![JavaType::Int]),
            2 => ("body", JavaType::Void, vec![JavaType::Int; 3]),
            _ => ("finish", JavaType::Void, vec![]),
        };
        Some(JavaMethod {
            owner: "Probe".into(),
            name: name.into(),
            is_static: true,
            return_type,
            parameters,
            throws: vec![],
        })
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: if reference.index() == 0 { "a" } else { "b" }.into(),
            ty: JavaType::Int,
            is_static: true,
        })
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Normal,
    FalsePolarity,
    Repeated,
    Missing,
    WrongPhi,
    MissingPhi,
    Exception,
}
fn fixture(mode: Mode) -> MethodIr<SsaForm> {
    let mut f = Fixture::new();
    let entry = f.cfg.entry();
    let seed = f.value(entry, ManagedType::Int);
    f.set(
        entry,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(seed),
            method: MethodRef::new(0),
            args: vec![],
        },
    );
    let store = f.block();
    f.set(
        store,
        ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value: seed,
        },
    );
    f.link(entry, store);
    let (tail, zero) = f.constant(store, 0);
    let (tail, one) = f.constant(tail, 1);
    let (tail, left) = f.constant(tail, 7);
    let (tail, right) = f.constant(tail, 11);
    let header = f.block();
    let hx = f.block();
    let hy = f.block();
    let n = f.value(header, ManagedType::Int);
    let x = f.value(hx, ManagedType::Int);
    let y = f.value(hy, ManagedType::Int);
    let initial = f.link(tail, header);
    f.link(header, hx);
    f.link(hx, hy);
    let evaluation = f.block();
    let condition = f.value(evaluation, ManagedType::Boolean);
    f.set(
        evaluation,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(condition),
            method: MethodRef::new(1),
            args: vec![n],
        },
    );
    f.link(hy, evaluation);
    let branch = f.block();
    let body = f.block();
    f.set(
        body,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![n, x, y],
        },
    );
    let suffix = f.block();
    let suffix_instruction = if matches!(mode, Mode::Missing) {
        ManagedInsn::Move {
            result: f.value(suffix, ManagedType::Int),
            value: x,
        }
    } else {
        ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(1),
            value: x,
        }
    };
    f.set(suffix, suffix_instruction);
    let mut selected = condition;
    if matches!(mode, Mode::FalsePolarity) {
        let negate = f.block();
        selected = f.value(negate, ManagedType::Boolean);
        // Compare with false reverses the original selected body edge.
        let false_block = f.block();
        let false_value = f.value(false_block, ManagedType::Boolean);
        f.set(
            false_block,
            ManagedInsn::Const {
                result: false_value,
                value: Constant::Boolean(false),
            },
        );
        f.link(evaluation, false_block);
        f.link(false_block, negate);
        f.set(
            negate,
            ManagedInsn::Compare {
                result: selected,
                op: CompareOp::Equal,
                lhs: condition,
                rhs: false_value,
            },
        );
        f.link(negate, branch);
    } else {
        f.link(evaluation, branch);
    }
    let yes_target = if matches!(mode, Mode::FalsePolarity) {
        suffix
    } else {
        body
    };
    let no_target = if matches!(mode, Mode::FalsePolarity) {
        body
    } else {
        suffix
    };
    let yes = f
        .cfg
        .add_edge(branch, yes_target, EdgeKind::BranchTrue)
        .unwrap();
    let no = f
        .cfg
        .add_edge(branch, no_target, EdgeKind::BranchFalse)
        .unwrap();
    f.set(
        branch,
        ManagedInsn::Branch {
            condition: Some(selected),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let mut last = body;
    if matches!(mode, Mode::Repeated) {
        let repeated = f.block();
        f.set(
            repeated,
            ManagedInsn::FieldPut {
                object: None,
                field: FieldRef::new(0),
                value: x,
            },
        );
        f.link(last, repeated);
        last = repeated;
    }
    let increment = f.block();
    let next = f.value(increment, ManagedType::Int);
    f.set(
        increment,
        ManagedInsn::Arithmetic {
            result: next,
            op: ArithOp::Add,
            lhs: n,
            rhs: one,
        },
    );
    f.link(last, increment);
    let back = f.link(increment, header);
    for (block, result, original, update) in
        [(header, n, zero, next), (hx, x, left, y), (hy, y, right, x)]
    {
        let mut inputs = vec![(initial, original), (back, update)];
        if matches!(mode, Mode::WrongPhi) {
            inputs[1].0 = yes;
        }
        if matches!(mode, Mode::MissingPhi) {
            inputs.pop();
        }
        f.set(block, ManagedInsn::Phi { result, inputs });
    }
    let finish = f.block();
    f.set(
        finish,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(3),
            args: vec![],
        },
    );
    f.link(suffix, finish);
    let ret = f.block();
    f.link(finish, ret);
    if matches!(mode, Mode::Exception) {
        f.cfg
            .add_edge(
                evaluation,
                ret,
                EdgeKind::Exception("Ljava/lang/RuntimeException;".into()),
            )
            .unwrap();
    }
    MethodIr::<SsaForm>::from_parts(f.cfg, f.instructions, f.values)
}
fn source(mode: Mode) -> Result<String, Unsupported> {
    emit_static_initializer_body_subset(
        &fixture(mode),
        &JavaMethod {
            owner: "Fixture".into(),
            name: "<clinit>".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        },
        &ParameterBindings::default(),
        &Resolver,
        &StaticInitializerContext {
            owner: "Fixture".into(),
            writable_static_fields: vec![],
        },
        &[FieldRef::new(0), FieldRef::new(1)],
    )
}
#[test]
fn canonical_loop_retains_final_stores_and_selected_phi_batches() {
    let body = source(Mode::Normal).unwrap();
    assert_eq!(body.matches("a = ").count(), 1);
    assert_eq!(body.matches("b = ").count(), 1);
    assert_eq!(body.matches("Probe.test(").count(), 1);
    assert!(body.contains("while (true)"));
    assert!(!body.contains("Fixture.a ="));
}
#[test]
fn repeated_missing_final_paths_invalid_phi_and_caught_edges_reject() {
    for mode in [
        Mode::Repeated,
        Mode::Missing,
        Mode::WrongPhi,
        Mode::MissingPhi,
        Mode::Exception,
    ] {
        assert!(source(mode).is_err());
    }
}
#[test]
#[ignore = "requires JDK17"]
fn compiled_original_and_lowered_loops_match_effects_reentrancy_phi_swap_and_throws() {
    use std::{path::PathBuf, process::Command};
    let home = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let root = std::env::temp_dir().join(format!("static-final-loops-{}", std::process::id()));
    let original = "static {a=Probe.seed(); int n=0,x=7,y=11; while(Probe.test(n)) {Probe.body(n,x,y); int t=x;x=y;y=t;n++;} b=x; Probe.finish();}";
    let probe = r#"public class Probe {
      static String trace="";
      static int seed(){trace+="seed:"+Fixture.a+","+Fixture.b+";";return 5;}
      static boolean test(int n){trace+="test:"+n+":"+Fixture.a+","+Fixture.b+";";if(n==Integer.getInteger("testThrow",-1))throw new IllegalStateException("test");return n<Integer.getInteger("limit",0);}
      static void body(int n,int x,int y){trace+="body:"+n+":"+x+","+y+";";if(n==Integer.getInteger("bodyThrow",-1))throw new IllegalArgumentException("body");}
      static void finish(){trace+="finish:"+Fixture.a+","+Fixture.b+";";}
      public static void main(String[]args){try{System.out.print(Fixture.a+","+Fixture.b+"|");}catch(ExceptionInInitializerError e){System.out.print(e.getCause().getClass().getName()+"|");}System.out.print(trace);}
    }"#;
    for (index, body) in [
        original.into(),
        source(Mode::Normal).unwrap(),
        source(Mode::FalsePolarity).unwrap(),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = root.join(index.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"),format!("public class Fixture {{public static final int a;public static final int b;{body}}}")).unwrap();
        std::fs::write(dir.join("Probe.java"), probe).unwrap();
        let output = Command::new(home.join("bin/javac"))
            .args(["--release", "17", "Fixture.java", "Probe.java"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{body}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for (limit, test_throw, body_throw) in [
        (0, -1, -1),
        (1, -1, -1),
        (2, -1, -1),
        (5, -1, -1),
        (3, 0, -1),
        (3, 2, -1),
        (3, -1, 0),
        (3, -1, 2),
    ] {
        let mut baseline = None;
        for index in 0..3 {
            let output = Command::new(home.join("bin/java"))
                .args([
                    format!("-Dlimit={limit}"),
                    format!("-DtestThrow={test_throw}"),
                    format!("-DbodyThrow={body_throw}"),
                ])
                .args(["-cp", ".", "Probe"])
                .current_dir(root.join(index.to_string()))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if let Some(expected) = &baseline {
                assert_eq!(&output.stdout, expected);
            } else {
                baseline = Some(output.stdout);
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
