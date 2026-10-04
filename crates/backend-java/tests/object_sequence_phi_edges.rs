//! Successful constructor identities are selected at exact predecessor exits.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
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
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture.Box".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: if r.index() == 0 {
                "Fixture.Box"
            } else {
                "Fixture"
            }
            .into(),
            name: if r.index() == 0 { "<init>" } else { "tick" }.into(),
            is_static: r.index() != 0,
            parameters: if r.index() == 0 {
                vec![JavaType::Int]
            } else {
                vec![]
            },
            return_type: if r.index() == 0 {
                JavaType::Void
            } else {
                JavaType::Int
            },
            throws: vec![],
        })
    }
}
fn def(
    i: &mut InsnArena,
    v: &mut ValueArena,
    t: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut out = None;
    let id = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, t));
        out = Some(x);
        f(x)
    });
    (id, out.unwrap())
}
fn emit(mode: u8) -> SourceResult {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let (p, flag) = def(&mut i, &mut v, ManagedType::Boolean, |result| {
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![result],
            effects: EffectFlags::default(),
        }
    });
    let branch = i.insert(ManagedInsn::Return { value: None });
    let (new, obj) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(0)),
        |result| ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        },
    );
    let (tick, n) = def(&mut i, &mut v, ManagedType::Int, |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(1),
            args: vec![],
        }
    });
    let init = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![obj, n],
    });
    let (null, nil) = def(&mut i, &mut v, ManagedType::Null, |result| {
        ManagedInsn::Const {
            result,
            value: Constant::Null,
        }
    });
    let (phi, result) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(0)),
        |result| ManagedInsn::Phi {
            result,
            inputs: vec![],
        },
    );
    let ret = i.insert(ManagedInsn::Return {
        value: Some(result),
    });
    let handler = i.insert(ManagedInsn::Opaque {
        family_tag: 1,
        reads: vec![],
        writes: vec![],
        effects: EffectFlags::default(),
    });
    let mut cfg = Cfg::new(p);
    let b = cfg.add_block(branch);
    let a = cfg.add_block(new);
    let t = cfg.add_block(tick);
    let c = cfg.add_block(init);
    let z = cfg.add_block(null);
    let h = (mode == 2).then(|| cfg.add_block(handler));
    let j = if mode == 3 {
        None
    } else {
        Some(cfg.add_block(phi))
    };
    let end = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), b, EdgeKind::Unconditional)
        .unwrap();
    let yes = cfg.add_edge(b, a, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(b, z, EdgeKind::BranchFalse).unwrap();
    i[branch] = ManagedInsn::Branch {
        condition: Some(flag),
        then_edge: yes,
        else_edge: Some(no),
    };
    cfg.add_edge(a, t, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(t, c, EdgeKind::Unconditional).unwrap();
    let target = j.unwrap_or(end);
    let left = match mode {
        1 => cfg.add_edge(a, target, EdgeKind::Unconditional).unwrap(),
        2 => {
            let h = h.unwrap();
            cfg.add_edge(c, h, EdgeKind::Exception("java.lang.Throwable".into()))
                .unwrap();
            cfg.add_edge(h, target, EdgeKind::Unconditional).unwrap()
        }
        4 => cfg
            .add_edge(c, target, EdgeKind::Exception("java.lang.Throwable".into()))
            .unwrap(),
        _ => cfg.add_edge(c, target, EdgeKind::Unconditional).unwrap(),
    };
    let right = cfg.add_edge(z, target, EdgeKind::Unconditional).unwrap();
    i[phi] = ManagedInsn::Phi {
        result,
        inputs: vec![(left, obj), (right, nil)],
    };
    if let Some(j) = j {
        cfg.add_edge(j, end, EdgeKind::Unconditional).unwrap();
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, i, v);
    backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "make".into(),
            is_static: true,
            parameters: vec![JavaType::Boolean],
            return_type: JavaType::Class("Fixture.Box".into()),
            throws: vec![],
        },
        &ParameterBindings {
            parameters: vec![flag],
            ..Default::default()
        },
        &R,
    )
}
#[test]
fn selected_normal_phi_preserves_constructor_effects_and_failure_identity() {
    let SourceResult::Complete { source, .. } = emit(0) else {
        panic!("{:?}", emit(0));
    };
    let root = std::env::temp_dir().join(format!("sequence-phi-edge-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Fixture.java"),format!(r#"public class Fixture {{static String log="";static int mode;static final RuntimeException FAIL=new RuntimeException();static class Box{{static{{log+="O";}}Box(int n){{log+="C";if(mode==2)throw FAIL;}}}}static int tick(){{log+="T";if(mode==1)throw FAIL;return 7;}}{source}public static void main(String[]a){{mode=Integer.parseInt(a[0]);if(make(false)!=null||!log.equals(""))throw new AssertionError(log);try{{if(make(true)==null||mode!=0||!log.equals("OTC"))throw new AssertionError(log);}}catch(RuntimeException e){{if(e!=FAIL||!log.equals(mode==1?"OT":"OTC"))throw new AssertionError(log);}}}}}}"#)).unwrap();
    let out = std::process::Command::new("javac")
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&out.stderr)
    );
    for mode in ["0", "1", "2"] {
        let out = std::process::Command::new("java")
            .args(["-Xverify:all", "Fixture", mode])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn preinitializer_exceptional_and_off_cfg_phi_reads_remain_denied() {
    for mode in 1..=4 {
        assert!(
            !matches!(emit(mode), SourceResult::Complete { .. }),
            "mode{mode}: {:?}",
            emit(mode)
        );
    }
}
