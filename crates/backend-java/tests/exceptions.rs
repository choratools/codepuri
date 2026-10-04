//! Compile the owned standalone modules without editing either crate root.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use backend_java::exceptions;
pub use backend_java::{JavaResolver, JavaType, branching};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::BlockId,
};
use exceptions::{
    ExceptionMetadata, TypedEmitter,
    scopes::{Catch, Scope},
};
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{Constant, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use std::{collections::HashMap, process::Command};

struct Metadata {
    scopes: Vec<Scope>,
    resolved: bool,
}
impl ExceptionMetadata for Metadata {
    fn scopes(&self) -> &[Scope] {
        &self.scopes
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        self.resolved.then(|| {
            JavaType::Class(
                match key {
                    "narrow" => "java.lang.ArithmeticException",
                    _ => "java.lang.RuntimeException",
                }
                .into(),
            )
        })
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        if actual == expected {
            return true;
        }
        let JavaType::Class(a) = actual else {
            return false;
        };
        let JavaType::Class(e) = expected else {
            return false;
        };
        matches!(
            (a.as_str(), e.as_str()),
            (
                "java.lang.ArithmeticException",
                "java.lang.RuntimeException" | "java.lang.Throwable"
            ) | ("java.lang.RuntimeException", "java.lang.Throwable")
        )
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.Throwable".into())
    }
    fn method(&self, r: MethodRef) -> Option<backend_java::JavaMethod> {
        Some(backend_java::JavaMethod {
            owner: "ExceptionRecovery".into(),
            name: match r.index() {
                0 => "work",
                1 => "reportNarrow",
                _ => "reportBroad",
            }
            .into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: if r.index() == 0 {
                vec![JavaType::Int; 2]
            } else {
                vec![JavaType::Class("java.lang.Throwable".into()), JavaType::Int]
            },
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
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
    fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), ty))
    }
    fn set(&mut self, b: BlockId, insn: ManagedInsn) {
        self.insns[*self.cfg.block(b).unwrap().payload()] = insn;
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> decomp_core::ids::EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
    fn constant(&mut self, b: BlockId, n: i32) -> ValueId {
        let result = self.value(b, ManagedType::Int);
        self.set(
            b,
            ManagedInsn::Const {
                result,
                value: Constant::Int(n),
            },
        );
        result
    }
}
fn fixture(bad_phi: bool) -> (MethodIr<SsaForm>, Metadata, ValueId) {
    let mut f = Builder::new();
    let incoming = f.insns.insert(ManagedInsn::Return { value: None });
    let mode = f.values.insert(Value::new(incoming, ManagedType::Int));
    f.insns[incoming] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![mode],
        effects: EffectFlags::NONE,
    };
    let entry = f.cfg.entry();
    let old = f.constant(entry, 10);
    let s1 = f.block();
    let one = f.constant(s1, 1);
    f.link(entry, s1);
    let s2 = f.block();
    let two = f.constant(s2, 2);
    f.link(s1, s2);
    let t1 = f.block();
    let r1 = f.value(t1, ManagedType::Int);
    f.set(
        t1,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(r1),
            method: MethodRef::new(0),
            args: vec![mode, one],
        },
    );
    f.link(s2, t1);
    let c1 = f.block();
    let committed = f.value(c1, ManagedType::Int);
    f.set(
        c1,
        ManagedInsn::Move {
            result: committed,
            value: r1,
        },
    );
    f.link(t1, c1);
    let t2 = f.block();
    let r2 = f.value(t2, ManagedType::Int);
    f.set(
        t2,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(r2),
            method: MethodRef::new(0),
            args: vec![mode, two],
        },
    );
    f.link(c1, t2);
    let c2 = f.block();
    let success = f.value(c2, ManagedType::Int);
    f.set(
        c2,
        ManagedInsn::Move {
            result: success,
            value: r2,
        },
    );
    f.link(t2, c2);
    // Tail deliberately allocated before head, as happens with prefix insertion.
    let phi_tail = f.block();
    let tail_output = f.value(phi_tail, ManagedType::Int);
    let join = f.block();
    let output = f.value(join, ManagedType::Int);
    let normal = f.link(c2, join);
    let mut catches = Vec::new();
    let mut join_inputs = vec![(normal, success)];
    for key in ["narrow", "broad"] {
        let head = f.block();
        let saved = f.value(head, ManagedType::Int);
        let e1 = f
            .cfg
            .add_edge(t1, head, EdgeKind::Exception(key.into()))
            .unwrap();
        let e2 = f
            .cfg
            .add_edge(t2, head, EdgeKind::Exception(key.into()))
            .unwrap();
        f.set(
            head,
            ManagedInsn::Phi {
                result: saved,
                inputs: vec![(e1, if bad_phi { r1 } else { old }), (e2, committed)],
            },
        );
        let tag = f.block();
        let exception = f.value(tag, ManagedType::Reference(ClassRef::new(1)));
        f.set(
            tag,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![exception],
                effects: EffectFlags::NONE,
            },
        );
        f.link(head, tag);
        let report = f.block();
        let result = f.value(report, ManagedType::Int);
        f.set(
            report,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(if key == "narrow" { 1 } else { 2 }),
                args: vec![exception, saved],
            },
        );
        f.link(tag, report);
        join_inputs.push((f.link(report, join), result));
        catches.push(Catch {
            key: key.into(),
            entry: head,
        });
    }
    f.set(
        join,
        ManagedInsn::Phi {
            result: output,
            inputs: join_inputs.clone(),
        },
    );
    f.set(
        phi_tail,
        ManagedInsn::Phi {
            result: tail_output,
            inputs: join_inputs,
        },
    );
    f.link(join, phi_tail);
    let ret = f.block();
    f.set(
        ret,
        ManagedInsn::Return {
            value: Some(tail_output),
        },
    );
    f.link(phi_tail, ret);
    (
        MethodIr::<SsaForm>::from_parts(f.cfg, f.insns, f.values),
        Metadata {
            scopes: vec![Scope {
                protected: vec![t1, c1, t2, c2],
                catches,
                continuation: join,
                has_unproven_finally: false,
            }],
            resolved: true,
        },
        mode,
    )
}
struct Emitter<'a> {
    ir: &'a MethodIr<SsaForm>,
    names: HashMap<ValueId, String>,
    source: String,
    next: usize,
}
impl Emitter<'_> {
    fn name(&self, v: ValueId) -> &str {
        self.names.get(&v).unwrap()
    }
}
impl TypedEmitter for Emitter<'_> {
    type Error = String;
    fn hoist(&mut self, locals: &[ValueId]) -> Result<(), String> {
        for (i, v) in locals.iter().enumerate() {
            let name = format!("v{i}");
            let (ty, init) = match self.ir.value(*v).unwrap().ty() {
                ManagedType::Int => ("int", "0"),
                ManagedType::Reference(_) => ("java.lang.Throwable", "null"),
                _ => return Err("type".into()),
            };
            self.source.push_str(&format!("{ty} {name} = {init};\n"));
            self.names.insert(*v, name);
        }
        Ok(())
    }
    fn instruction(&mut self, id: InsnId) -> Result<(), String> {
        let line = match self.ir.instruction(id).unwrap() {
            ManagedInsn::Const {
                result,
                value: Constant::Int(n),
            } => format!("{} = {n};", self.name(*result)),
            ManagedInsn::Move { result, value } => {
                format!("{} = {};", self.name(*result), self.name(*value))
            }
            ManagedInsn::Invoke {
                result: Some(result),
                method,
                args,
                ..
            } => {
                let name = match method.index() {
                    0 => "work",
                    1 => "reportNarrow",
                    _ => "reportBroad",
                };
                format!(
                    "{} = {name}({});",
                    self.name(*result),
                    args.iter()
                        .map(|v| self.name(*v))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            ManagedInsn::Return { value: Some(v) } => format!("return {};", self.name(*v)),
            _ => return Err("instruction".into()),
        };
        self.source.push_str(&line);
        self.source.push('\n');
        Ok(())
    }
    fn copies(&mut self, copies: &[branching::Copy]) -> Result<(), String> {
        let mut temps = Vec::new();
        for c in copies {
            let t = format!("edge{}", self.next);
            self.next += 1;
            self.source
                .push_str(&format!("int {t} = {};\n", self.name(c.source)));
            temps.push(t);
        }
        for (c, t) in copies.iter().zip(temps) {
            self.source
                .push_str(&format!("{} = {t};\n", self.name(c.destination)));
        }
        Ok(())
    }
    fn begin_try(&mut self) -> Result<(), String> {
        self.source.push_str("try {\n");
        Ok(())
    }
    fn begin_catch(&mut self, ty: &JavaType, exception: ValueId) -> Result<(), String> {
        let JavaType::Class(class) = ty else {
            return Err("catch type".into());
        };
        self.source.push_str(&format!(
            "}} catch ({class} caught) {{\n{} = caught;\n",
            self.name(exception)
        ));
        Ok(())
    }
    fn end_catches(&mut self) -> Result<(), String> {
        self.source.push_str("}\n");
        Ok(())
    }
}

#[test]
fn javac_runtime_beforethrow_success_and_ordered_typed_catches() {
    let (ir, metadata, mode) = fixture(false);
    let plan = exceptions::plan(&ir, &metadata, &[mode], &Resolver).unwrap();
    let mut emitter = Emitter {
        ir: &ir,
        names: HashMap::from([(mode, "mode".into())]),
        source: String::new(),
        next: 0,
    };
    exceptions::emit(&plan, &mut emitter).unwrap();
    let production = backend_java::emit_method_declaration_with_exceptions(
        &ir,
        &backend_java::JavaMethod {
            owner: "ExceptionRecovery".into(),
            name: "recovered".into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Int],
            throws: vec![],
        },
        &backend_java::DeclarationBindings {
            parameters: vec![Some(mode)],
            ..Default::default()
        },
        &Resolver,
        Some(&metadata),
    );
    let backend_java::SourceResult::Complete {
        source: production, ..
    } = production
    else {
        panic!("{production:?}");
    };
    assert_eq!(production.matches("ExceptionRecovery.work(").count(), 2);
    assert_eq!(emitter.source.matches("work(").count(), 2);
    assert!(
        emitter
            .source
            .find("catch (java.lang.ArithmeticException")
            .unwrap()
            < emitter
                .source
                .find("catch (java.lang.RuntimeException")
                .unwrap()
    );
    let source = format!(
        r#"public class ExceptionRecovery {{
      static int work(int mode,int stage) {{
        if (mode==1 && stage==1 || mode==3 && stage==2) throw new ArithmeticException();
        if (mode==2 && stage==2) throw new IllegalArgumentException();
        if (mode==4 && stage==2) throw new RuntimeException();
        return stage==1 ? 20 : 30;
      }}
      static int reportNarrow(Throwable ex,int saved) {{
        if (!(ex instanceof ArithmeticException)) throw new AssertionError("wrong caught value");
        return saved + 100;
      }}
      static int reportBroad(Throwable ex,int saved) {{
        if (!(ex instanceof RuntimeException) || ex instanceof ArithmeticException) throw new AssertionError("wrong catch dispatch/value");
        return saved + 200;
      }}
      {}
      public static void main(String[] args) {{
        int[] expected={{30,110,220,120,220}};
        for(int i=0;i<expected.length;i++) if(recovered(i)!=expected[i]) throw new AssertionError(i+":"+recovered(i));
        System.out.println("OK beforethrow/success/catch-order");
      }}
    }}"#,
        production
    );
    let dir = std::env::temp_dir().join(format!(
        "decomphify-exceptions-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("ExceptionRecovery.java"), &source).unwrap();
    let compilation = Command::new("javac")
        .arg("ExceptionRecovery.java")
        .current_dir(&dir)
        .output()
        .expect("javac required for runtime evidence");
    assert!(
        compilation.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let execution = Command::new("java")
        .args(["-cp", ".", "ExceptionRecovery"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(
        String::from_utf8_lossy(&execution.stdout).contains("OK beforethrow/success/catch-order")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
#[test]
fn throwing_result_is_unavailable_on_exception_edge() {
    let (ir, m, b) = fixture(true);
    assert!(matches!(
        exceptions::plan(&ir, &m, &[b], &Resolver),
        Err(exceptions::Error::Unavailable(_))
    ));
}
#[test]
fn reject_finally_overlap_missing_types_and_shadowing_order() {
    let (ir, mut m, b) = fixture(false);
    m.scopes[0].has_unproven_finally = true;
    assert!(exceptions::plan(&ir, &m, &[b], &Resolver).is_err());
    m.scopes[0].has_unproven_finally = false;
    m.scopes.push(m.scopes[0].clone());
    assert!(exceptions::plan(&ir, &m, &[b], &Resolver).is_err());
    m.scopes.pop();
    m.resolved = false;
    assert!(exceptions::plan(&ir, &m, &[b], &Resolver).is_err());
    m.resolved = true;
    m.scopes[0].catches.reverse();
    assert!(exceptions::plan(&ir, &m, &[b], &Resolver).is_err());
}
#[test]
fn reject_noncontiguous_scope() {
    let (ir, mut m, b) = fixture(false);
    m.scopes[0].protected.remove(1);
    assert!(exceptions::plan(&ir, &m, &[b], &Resolver).is_err());
}

#[test]
fn checked_allowances_apply_only_to_protected_instructions() {
    struct CheckedMetadata(Metadata);
    impl ExceptionMetadata for CheckedMetadata {
        fn scopes(&self) -> &[Scope] {
            &self.0.scopes
        }
        fn catch_type(&self, key: &str) -> Option<JavaType> {
            Some(JavaType::Class(
                if key == "narrow" {
                    "java.io.IOException"
                } else {
                    "java.lang.Exception"
                }
                .into(),
            ))
        }
    }
    struct CheckedResolver(bool);
    impl JavaResolver for CheckedResolver {
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            a == b
                || matches!((a,b), (JavaType::Class(a),JavaType::Class(b))
                if (a == "java.io.IOException" && (b == "java.lang.Exception" || b == "java.lang.Throwable"))
                || (a == "java.lang.Exception" && b == "java.lang.Throwable"))
        }
        fn method(&self, r: MethodRef) -> Option<backend_java::JavaMethod> {
            let mut m = Resolver.method(r)?;
            if r.index() == 0 || self.0 {
                m.throws = vec!["java.io.IOException".into()];
            }
            Some(m)
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Resolver.class(r)
        }
        fn field(&self, r: FieldRef) -> Option<backend_java::JavaField> {
            Resolver.field(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            Resolver.string(r)
        }
    }
    let (ir, metadata, mode) = fixture(false);
    let metadata = CheckedMetadata(metadata);
    let m = backend_java::JavaMethod {
        owner: "ExceptionRecovery".into(),
        name: "recovered".into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![JavaType::Int],
        throws: vec![],
    };
    let bindings = backend_java::DeclarationBindings {
        parameters: vec![Some(mode)],
        ..Default::default()
    };
    assert!(matches!(
        backend_java::emit_method_declaration_with_exceptions(
            &ir,
            &m,
            &bindings,
            &CheckedResolver(false),
            Some(&metadata)
        ),
        backend_java::SourceResult::Complete { .. }
    ));
    let rejected = backend_java::emit_method_declaration_with_exceptions(
        &ir,
        &m,
        &bindings,
        &CheckedResolver(true),
        Some(&metadata),
    );
    assert!(
        matches!(rejected, backend_java::SourceResult::Unsupported(backend_java::Unsupported::Metadata(ref reason)) if reason.contains("checked exceptions")),
        "{rejected:?}"
    );
}
