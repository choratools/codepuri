//! Captured values are available at selected normal Phi edges only.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::method::{InsnArena, ValueArena};
use managed_ir::types::ManagedType;
use managed_ir::value::Value;
struct R;
impl JavaResolver for R {
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
#[test]
fn captured_phi_requires_definition_and_success_on_selected_edge() {
    for mode in 0..=5 {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let mut object = None;
        let allocation = insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
            object = Some(v);
            ManagedInsn::NewInstance {
                result: v,
                class: ClassRef::new(0),
            }
        });
        let object = object.unwrap();
        let mut captured = None;
        let definition = insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Int));
            captured = Some(v);
            ManagedInsn::Const {
                result: v,
                value: Constant::Int(7),
            }
        });
        let captured = captured.unwrap();
        let initializer = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![object],
        });
        let other = insns.insert(ManagedInsn::Return { value: None });
        let handler = insns.insert(ManagedInsn::Return { value: None });
        let mut out = None;
        let phi = insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Int));
            out = Some(value);
            ManagedInsn::Phi {
                result: value,
                inputs: vec![],
            }
        });
        let start = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(start);
        let a = cfg.add_block(allocation);
        let d = cfg.add_block(definition);
        let c = cfg.add_block(initializer);
        let o = cfg.add_block(other);
        let h = cfg.add_block(handler);
        let p = (mode != 4).then(|| cfg.add_block(phi));
        let target = p.unwrap_or(o);
        cfg.add_edge(cfg.entry(), a, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(cfg.entry(), o, EdgeKind::BranchFalse).unwrap();
        if mode == 5 {
            cfg.add_edge(a, c, EdgeKind::Unconditional).unwrap();
        } else {
            cfg.add_edge(a, d, EdgeKind::Unconditional).unwrap();
            cfg.add_edge(d, c, EdgeKind::Unconditional).unwrap();
        }
        let edge = match mode {
            1 => cfg.add_edge(a, target, EdgeKind::Unconditional).unwrap(),
            2 => cfg
                .add_edge(c, target, EdgeKind::Exception("E".into()))
                .unwrap(),
            3 => {
                cfg.add_edge(c, h, EdgeKind::Exception("E".into())).unwrap();
                cfg.add_edge(h, target, EdgeKind::Unconditional).unwrap()
            }
            _ => cfg.add_edge(c, target, EdgeKind::Unconditional).unwrap(),
        };
        insns[phi] = ManagedInsn::Phi {
            result: out.unwrap(),
            inputs: vec![(edge, captured)],
        };
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let e = Emitter {
            ir: &ir,
            resolver: &R,
            values: HashMap::new(),
            lines: vec![],
            next: 0,
            parameters: HashSet::new(),
            reserved: HashSet::new(),
            current: None,
            hoisted: HashSet::new(),
            declarations: HashMap::new(),
            allocation_readable: None,
            object_planning: Default::default(),
        };
        let after = e.after_initializer(initializer).unwrap();
        assert_eq!(
            e.initialized_capture_use(phi, captured, &after).unwrap(),
            mode == 0,
            "mode{mode}"
        );
    }
}

#[test]
fn selected_capture_branch_preserves_new_effect_failure_and_phi_result() {
    let dir = std::env::temp_dir().join(format!("selected-capture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Capture.java"), r#"public class Capture {
      static int mode; static String log="";
      static final RuntimeException failure=new IllegalStateException("same");
      static int tick() {log+="T";if(mode==1)throw failure;return 7;}
      static int other() {log+="B";return 9;}
      static class Owner {static {log+="N";}Owner(int n){log+="C";if(mode==2)throw failure;if(n!=7)throw new AssertionError();}}
      static int original(boolean selected) {int capture=0;if(selected){new Owner(capture=tick());return capture;}else{return other();}}
      static int lowered(boolean selected) {int capture=0;if(selected){new Owner(switch(0){default->{int v=tick();capture=v;yield v;}});return capture;}else{return other();}}
      public static void main(String[] a){mode=Integer.parseInt(a[1]);try{int n=a[0].equals("original")?original(mode!=3):lowered(mode!=3);System.out.println(n+":"+log);}catch(RuntimeException e){if(e!=failure)throw new AssertionError("identity");System.out.println("failure:"+log);}}
    }"#).unwrap();
    for home in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let compiled = std::process::Command::new(format!("{home}/bin/javac"))
            .args(["Capture.java"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        for (mode, expected) in [
            ("0", "7:NTC\n"),
            ("1", "failure:NT\n"),
            ("2", "failure:NTC\n"),
            ("3", "9:B\n"),
        ] {
            for method in ["original", "lowered"] {
                let out = std::process::Command::new(format!("{home}/bin/java"))
                    .args(["-cp", ".", "Capture", method, mode])
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert_eq!(
                    String::from_utf8(out.stdout).unwrap(),
                    expected,
                    "{home} {method} {mode}"
                );
            }
        }
    }
}
