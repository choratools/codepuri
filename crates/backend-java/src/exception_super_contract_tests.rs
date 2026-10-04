//! Native catch reachability uses the emitted super target, not pool metadata.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    JavaMethod, SuperInvocation,
    type_compatibility::{Declaration, Hierarchy},
};
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
};
struct Resolver(Option<bool>);
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Base".into(),
            name: "callback".into(),
            is_static: false,
            parameters: vec![],
            return_type: JavaType::Void,
            throws: vec!["java.io.IOException".into()],
        })
    }
    fn super_invocation(&self, caller: &str, r: MethodRef) -> Option<SuperInvocation> {
        if caller != "Child" {
            return None;
        }
        let checked = self.0?;
        let mut method = self.method(r)?;
        if !checked {
            method.owner = "Empty".into();
            method.throws.clear();
        }
        Some(SuperInvocation {
            method,
            access_flags: 1,
        })
    }
}
fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
    ] {
        h.insert(Declaration {
            name: name.into(),
            superclass: parent.map(str::to_owned),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}
#[test]
fn super_native_checked_catch_requires_selected_source_contract() {
    let mut insns = InsnArena::with_key();
    let id = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Super,
        method: MethodRef::new(0),
        args: vec![],
        result: None,
    });
    let ir = MethodIr::<SsaForm>::from_parts(
        decomp_core::cfg::Cfg::new(id),
        insns,
        ValueArena::with_key(),
    );
    let insn = ManagedInsn::Invoke {
        kind: InvokeKind::Super,
        method: MethodRef::new(0),
        args: vec![],
        result: None,
    };
    let caught = JavaType::Class("java.io.IOException".into());
    for selected in [Some(false), None, Some(true)] {
        let resolver = Resolver(selected);
        let emitter = crate::Emitter {
            allocation_readable: None,
            object_planning: Default::default(),
            ir: &ir,
            resolver: &resolver,
            values: HashMap::new(),
            lines: vec![],
            next: 0,
            parameters: HashSet::new(),
            reserved: HashSet::new(),
            current: None,
            hoisted: HashSet::new(),
            declarations: HashMap::new(),
        };
        let result = native_catch_reachable(&emitter, "Child", id, &insn, &caught, &hierarchy());
        match selected {
            Some(false) => assert!(matches!(result, Ok(false))),
            None => assert!(result.is_err()),
            Some(true) => assert!(matches!(result, Ok(true))),
        }
        // The existing unchecked-catch language rule remains independent of a missing source contract.
        assert!(
            native_catch_reachable(
                &emitter,
                "Child",
                id,
                &insn,
                &JavaType::Class("java.lang.RuntimeException".into()),
                &hierarchy()
            )
            .unwrap()
        );
    }
}
#[test]
fn genuine_overridden_super_contract_controls_javac_reachability() {
    use std::{fs, process::Command};
    let dir = std::env::temp_dir().join(format!("decomphify-super-catch-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let declarations = "class Base { void callback() throws java.io.IOException {} } class Empty extends Base { @Override void callback() {} }";
    let good = format!(
        "{declarations} class Good extends Base {{ void recovered() {{try{{super.callback();}}catch(java.io.IOException caught){{}}}} }}"
    );
    fs::write(dir.join("Good.java"), good).unwrap();
    assert!(
        Command::new("javac")
            .arg(dir.join("Good.java"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let bad = format!(
        "{declarations} class Bad extends Empty {{ void rejected() {{try{{super.callback();}}catch(java.io.IOException caught){{}}}} }}"
    );
    fs::write(dir.join("Bad.java"), bad).unwrap();
    let out = Command::new("javac")
        .arg(dir.join("Bad.java"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("never thrown"));
    fs::remove_dir_all(dir).unwrap();
}
