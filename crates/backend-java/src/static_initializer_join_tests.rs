//! Edge-specific Phi joins and original final-store paths.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture".into(),
            name: match reference.index() {
                0 => "choose",
                2 => "chooseAgain",
                _ => "capture",
            }
            .into(),
            is_static: true,
            return_type: if matches!(reference.index(), 0 | 2) {
                JavaType::Boolean
            } else {
                JavaType::Void
            },
            parameters: vec![],
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
fn signature() -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: "<clinit>".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: vec![],
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Join,
    Shared,
    Missing,
    Double,
    WrongPhi,
    MissingPhi,
    DuplicatePhi,
    EarlyReturn,
}
fn fixture(mode: Mode) -> MethodIr<SsaForm> {
    let mut instructions = InsnArena::default();
    let mut values = ValueArena::default();
    let mut condition = None;
    let choose = instructions.insert_with_key(|id| {
        let value = values.insert(Value::new(id, ManagedType::Boolean));
        condition = Some(value);
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(value),
            method: MethodRef::new(0),
            args: vec![],
        }
    });
    let mut cfg = Cfg::new(choose);
    let branch_id = instructions.insert(ManagedInsn::Opaque {
        family_tag: 0,
        reads: vec![],
        writes: vec![],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let branch = cfg.add_block(branch_id);
    cfg.add_edge(cfg.entry(), branch, EdgeKind::Unconditional)
        .unwrap();
    let mut constant = |number: i32| {
        let mut value = None;
        let id = instructions.insert_with_key(|id| {
            let result = values.insert(Value::new(id, ManagedType::Int));
            value = Some(result);
            ManagedInsn::Const {
                result,
                value: Constant::Int(number),
            }
        });
        (cfg.add_block(id), value.unwrap())
    };
    let (yes_a, ya) = constant(7);
    let (yes_b, yb) = constant(11);
    let (no_a, na) = constant(11);
    let (no_b, nb) = constant(7);
    let yes = cfg.add_edge(branch, yes_a, EdgeKind::BranchTrue).unwrap();
    let false_target = if matches!(mode, Mode::Shared) {
        let mut second_condition = None;
        let call = instructions.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Boolean));
            second_condition = Some(value);
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(value),
                method: MethodRef::new(2),
                args: vec![],
            }
        });
        let call = cfg.add_block(call);
        let inner_id = instructions.insert(ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
        let inner = cfg.add_block(inner_id);
        cfg.add_edge(call, inner, EdgeKind::Unconditional).unwrap();
        let inner_yes = cfg.add_edge(inner, yes_a, EdgeKind::BranchTrue).unwrap();
        let inner_no = cfg.add_edge(inner, no_a, EdgeKind::BranchFalse).unwrap();
        instructions[inner_id] = ManagedInsn::Branch {
            condition: second_condition,
            then_edge: inner_yes,
            else_edge: Some(inner_no),
        };
        call
    } else {
        no_a
    };
    let no = cfg
        .add_edge(branch, false_target, EdgeKind::BranchFalse)
        .unwrap();
    instructions[branch_id] = ManagedInsn::Branch {
        condition,
        then_edge: yes,
        else_edge: Some(no),
    };
    cfg.add_edge(yes_a, yes_b, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(no_a, no_b, EdgeKind::Unconditional).unwrap();
    let capture = cfg.add_block(instructions.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(1),
        args: vec![],
    }));
    cfg.add_edge(yes_b, capture, EdgeKind::Unconditional)
        .unwrap();
    let phi1_id = instructions.insert(ManagedInsn::Opaque {
        family_tag: 0,
        reads: vec![],
        writes: vec![],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let phi1 = cfg.add_block(phi1_id);
    let left = if matches!(mode, Mode::EarlyReturn) {
        let put_a = cfg.add_block(instructions.insert(ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value: ya,
        }));
        let put_b = cfg.add_block(instructions.insert(ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(1),
            value: yb,
        }));
        let ret = cfg.add_block(instructions.insert(ManagedInsn::Return { value: None }));
        cfg.add_edge(capture, put_a, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(put_a, put_b, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(put_b, ret, EdgeKind::Unconditional).unwrap();
        None
    } else {
        let tail = if matches!(mode, Mode::Double) {
            let put = cfg.add_block(instructions.insert(ManagedInsn::FieldPut {
                object: None,
                field: FieldRef::new(0),
                value: ya,
            }));
            cfg.add_edge(capture, put, EdgeKind::Unconditional).unwrap();
            put
        } else {
            capture
        };
        Some(cfg.add_edge(tail, phi1, EdgeKind::Unconditional).unwrap())
    };
    let right = cfg.add_edge(no_b, phi1, EdgeKind::Unconditional).unwrap();
    let mut first_inputs = vec![(right, na)];
    let mut second_inputs = vec![(right, nb)];
    if let Some(left) = left {
        first_inputs.push((left, ya));
        second_inputs.push((left, yb));
    }
    if matches!(mode, Mode::WrongPhi) {
        first_inputs[0].0 = yes;
    }
    if matches!(mode, Mode::MissingPhi) {
        first_inputs.pop();
    }
    if matches!(mode, Mode::DuplicatePhi) {
        first_inputs[1].0 = first_inputs[0].0;
    }
    let first = values.insert(Value::new(phi1_id, ManagedType::Int));
    instructions[phi1_id] = ManagedInsn::Phi {
        result: first,
        inputs: first_inputs,
    };
    let mut second = None;
    let phi2_id = instructions.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Int));
        second = Some(result);
        ManagedInsn::Phi {
            result,
            inputs: second_inputs,
        }
    });
    let phi2 = cfg.add_block(phi2_id);
    cfg.add_edge(phi1, phi2, EdgeKind::Unconditional).unwrap();
    let mut tail = phi2;
    let mut suffix = vec![ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(0),
        value: first,
    }];
    if !matches!(mode, Mode::Missing) {
        suffix.push(ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(1),
            value: second.unwrap(),
        });
    }
    suffix.extend([
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![],
        },
        ManagedInsn::Return { value: None },
    ]);
    for instruction in suffix {
        let next = cfg.add_block(instructions.insert(instruction));
        cfg.add_edge(tail, next, EdgeKind::Unconditional).unwrap();
        tail = next;
    }
    MethodIr::<SsaForm>::from_parts(cfg, instructions, values)
}
fn source(mode: Mode) -> Result<String, Unsupported> {
    emit_static_initializer_body_subset(
        &fixture(mode),
        &signature(),
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
fn shared_final_stores_use_edge_specific_parallel_phi_copies() {
    let result = source(Mode::Join).unwrap();
    assert_eq!(result.matches("a = ").count(), 1);
    assert_eq!(result.matches("b = ").count(), 1);
    assert!(!result.contains("Fixture.a ="));
    assert!(!result.contains("return;"));
    assert!(result.contains("break staticInitializerBody;"));
    assert_eq!(result.matches("Fixture.capture();").count(), 2);
}
#[test]
fn missing_double_and_wrong_original_phi_edges_fail_closed() {
    for mode in [
        Mode::Missing,
        Mode::Double,
        Mode::WrongPhi,
        Mode::MissingPhi,
        Mode::DuplicatePhi,
    ] {
        assert!(source(mode).is_err());
    }
}
#[test]
#[ignore = "requires JDK17"]
fn java_final_assignment_and_reentrant_observations_match_both_arms_and_early_return() {
    use std::{path::PathBuf, process::Command};
    let java_home = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (mode_index, mode) in [Mode::Join, Mode::EarlyReturn, Mode::Shared]
        .into_iter()
        .enumerate()
    {
        let body = source(mode).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "static-final-joins-{}-{}",
            std::process::id(),
            mode_index
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let unit = format!(
            "public class Fixture {{ public static final int a; public static final int b; public static String trace = \"\"; static boolean choose() {{ trace += \"choose:\" + a + \",\" + b + \";\"; if (Boolean.getBoolean(\"fail\")) throw new IllegalStateException(\"before stores\"); return Boolean.getBoolean(\"yes\"); }} static boolean chooseAgain() {{ trace += \"again:\" + a + \",\" + b + \";\"; return Boolean.getBoolean(\"second\"); }} static void capture() {{ trace += a + \",\" + b + \";\"; }} {body} public static void main(String[] args) {{ System.out.print(a + \",\" + b + \":\" + trace); }} }}"
        );
        std::fs::write(dir.join("Fixture.java"), unit).unwrap();
        let compile = Command::new(java_home.join("bin/javac"))
            .args(["--release", "17", "Fixture.java"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}\n{body}",
            String::from_utf8_lossy(&compile.stderr)
        );
        for (yes, second) in [(false, false), (false, true), (true, false), (true, true)] {
            let run = Command::new(java_home.join("bin/java"))
                .arg(format!("-Dyes={yes}"))
                .arg(format!("-Dsecond={second}"))
                .args(["-cp", ".", "Fixture"])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            let expected = if matches!(mode, Mode::Shared) && !yes {
                if second {
                    "7,11:choose:0,0;again:0,0;0,0;7,11;"
                } else {
                    "11,7:choose:0,0;again:0,0;11,7;"
                }
            } else if yes {
                if matches!(mode, Mode::EarlyReturn) {
                    "7,11:choose:0,0;0,0;"
                } else {
                    "7,11:choose:0,0;0,0;7,11;"
                }
            } else {
                "11,7:choose:0,0;11,7;"
            };
            assert_eq!(String::from_utf8(run.stdout).unwrap(), expected);
        }
        let throwing = Command::new(java_home.join("bin/java"))
            .args(["-Dfail=true", "-cp", ".", "Fixture"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(!throwing.status.success());
        assert!(String::from_utf8_lossy(&throwing.stderr).contains("before stores"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
