//! Original-order eager effects in Java25's early construction context.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{ArithOp, Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use std::{path::PathBuf, process::Command};

struct R {
    release: u16,
    checked: bool,
    constant_field: bool,
    selected_super: bool,
}
fn good() -> R {
    R {
        release: 25,
        checked: false,
        constant_field: false,
        selected_super: true,
    }
}
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "Fixture",
                1 => "Probe",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        Some(JavaType::Byte)
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: if f.index() == 2 { "Fixture" } else { "Probe" }.into(),
            name: match f.index() {
                0 => "shared",
                1 => "value",
                _ => "captured",
            }
            .into(),
            ty: JavaType::Int,
            is_static: f.index() == 0,
        })
    }
    fn field_access(&self, f: FieldRef) -> Option<u32> {
        Some(match f.index() {
            0 => {
                if self.constant_field {
                    0x19
                } else {
                    0x49
                }
            }
            1 => 1,
            _ => 0x11,
        })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() < 2 {
            return self.initializer(m).map(|i| i.method);
        }
        if m.index() == 5 {
            return Some(JavaMethod {
                owner: "Fixture".into(),
                name: "select".into(),
                parameters: vec![JavaType::Int],
                return_type: JavaType::Int,
                is_static: false,
                throws: vec!["java.io.IOException".into()],
            });
        }
        Some(JavaMethod {
            owner: "Probe".into(),
            name: match m.index() {
                2 => "mark",
                3 => "combine",
                _ => "read",
            }
            .into(),
            parameters: match m.index() {
                2 => vec![JavaType::Int],
                3 => vec![JavaType::Int, JavaType::Int],
                _ => vec![],
            },
            return_type: if m.index() == 2 {
                JavaType::Void
            } else {
                JavaType::Int
            },
            is_static: m.index() != 4,
            throws: if self.checked && m.index() == 3 {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
    fn method_access(&self, m: MethodRef) -> Option<u32> {
        Some(if m.index() == 4 { 1 } else { 9 })
    }
    fn super_invocation(
        &self,
        caller: &str,
        reference: MethodRef,
    ) -> Option<backend_java::SuperInvocation> {
        (self.selected_super && caller == "Fixture" && reference.index() == 5).then(|| {
            backend_java::SuperInvocation {
                method: JavaMethod {
                    owner: "Base".into(),
                    name: "select".into(),
                    parameters: vec![JavaType::Int],
                    return_type: JavaType::Int,
                    is_static: false,
                    throws: if self.checked {
                        vec!["java.io.IOException".into()]
                    } else {
                        vec![]
                    },
                },
                access_flags: 1,
            }
        })
    }
}
impl ConstructorResolver for R {
    fn constructor_source_level(&self) -> u16 {
        self.release
    }
    fn constructor_early_field_store(&self, f: FieldRef) -> bool {
        f.index() == 2
    }
    fn constructor_super_target(&self, _: &str, reference: MethodRef) -> bool {
        reference.index() == 5
    }
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        if m.index() > 1 {
            return None;
        }
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 0 { "Fixture" } else { "Base" }.into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![
                        JavaType::Class("Probe".into()),
                        JavaType::Int,
                        JavaType::Int,
                    ]
                } else {
                    vec![JavaType::Int]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
struct Build {
    insns: InsnArena,
    values: ValueArena,
    ids: Vec<InsnId>,
}
impl Build {
    fn new() -> Self {
        Self {
            insns: Default::default(),
            values: Default::default(),
            ids: vec![],
        }
    }
    fn value(
        &mut self,
        ty: ManagedType,
        f: impl FnOnce(ValueId) -> ManagedInsn,
        cfg: bool,
    ) -> ValueId {
        let mut result = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            result = Some(v);
            f(v)
        });
        if cfg {
            self.ids.push(id);
        }
        result.unwrap()
    }
    fn parameter(&mut self, ty: ManagedType) -> ValueId {
        self.value(
            ty,
            |v| ManagedInsn::Opaque {
                family_tag: 0,
                reads: vec![],
                writes: vec![v],
                effects: EffectFlags::NONE,
            },
            false,
        )
    }
    fn push(&mut self, i: ManagedInsn) {
        self.ids.push(self.insns.insert(i));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.ids[0]);
        let mut prev = cfg.entry();
        for id in &self.ids[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(prev, next, EdgeKind::Unconditional).unwrap();
            prev = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
fn body(bad: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Build::new();
    let this = b.parameter(ManagedType::Reference(ClassRef::new(0)));
    let other = b.parameter(ManagedType::Reference(ClassRef::new(1)));
    let length = b.parameter(ManagedType::Int);
    let divisor = b.parameter(ManagedType::Int);
    let one = b.value(
        ManagedType::Int,
        |result| ManagedInsn::Const {
            result,
            value: Constant::Int(1),
        },
        true,
    );
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        method: MethodRef::new(2),
        result: None,
        args: vec![if bad == 1 { this } else { one }],
    });
    let array = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Byte),
            nullable: false,
        },
        |result| ManagedInsn::NewArray {
            result,
            element: TypeRef::new(0),
            length,
        },
        true,
    );
    let size = b.value(
        ManagedType::Int,
        |result| ManagedInsn::ArrayLength { result, array },
        true,
    );
    let quotient = b.value(
        ManagedType::Int,
        |result| ManagedInsn::Arithmetic {
            op: ArithOp::Div,
            result,
            lhs: size,
            rhs: divisor,
        },
        true,
    );
    let shared = b.value(
        ManagedType::Int,
        |result| ManagedInsn::FieldGet {
            result,
            object: None,
            field: FieldRef::new(0),
        },
        true,
    );
    b.push(ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(0),
        value: quotient,
    });
    let read = b.value(
        ManagedType::Int,
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: Some(result),
            method: MethodRef::new(4),
            args: vec![if bad == 2 { this } else { other }],
        },
        true,
    );
    let value = b.value(
        ManagedType::Int,
        |result| ManagedInsn::FieldGet {
            result,
            object: Some(if bad == 3 { this } else { other }),
            field: FieldRef::new(1),
        },
        true,
    );
    let sum = b.value(
        ManagedType::Int,
        |result| ManagedInsn::Arithmetic {
            op: ArithOp::Add,
            result,
            lhs: shared,
            rhs: read,
        },
        true,
    );
    let combined = b.value(
        ManagedType::Int,
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(3),
            args: vec![sum, value],
        },
        true,
    );
    b.push(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(2),
        value: combined,
    });
    let two = b.value(
        ManagedType::Int,
        |result| ManagedInsn::Const {
            result,
            value: Constant::Int(2),
        },
        true,
    );
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(2),
        args: vec![two],
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this, combined],
    });
    b.push(ManagedInsn::Return { value: None });
    (
        b.finish(),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![other, length, divisor],
            names: vec![],
        },
    )
}
#[test]
fn source25_materializes_effects_and_duplicate_results_before_chain() {
    let (ir, bindings) = body(0);
    let p = plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &good()).unwrap();
    assert!(p.prologue.iter().any(|l| l.contains("new byte[p1]")));
    assert!(p.prologue.iter().any(|l| l.contains(" / ")));
    assert_eq!(
        p.prologue
            .iter()
            .filter(|l| l.contains("Probe.combine("))
            .count(),
        1
    );
    assert!(!p.first_statement.contains("combine"));
    assert!(p.source.find("Probe.shared =").unwrap() < p.source.find("super(").unwrap());
    assert!(p.source.find("this.captured =").unwrap() < p.source.find("super(").unwrap());
    assert_eq!(p.preserved_pre_chain_fields.len(), 1);
    let mut r = good();
    r.release = 17;
    assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &r).is_err());
}
#[test]
fn source25_rejects_this_uses_missing_exception_and_constant_field_proof() {
    for bad in 1..=3 {
        let (ir, b) = body(bad);
        assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &b, &good()).is_err());
    }
    let (ir, b) = body(0);
    let mut r = good();
    r.checked = true;
    assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &b, &r).is_err());
    r = good();
    r.constant_field = true;
    assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &b, &r).is_err());
}

#[test]
fn constructor_suffix_super_uses_selected_metadata_before_exception_checks() {
    let mut b = Build::new();
    let this = b.parameter(ManagedType::Reference(ClassRef::new(0)));
    let other = b.parameter(ManagedType::Reference(ClassRef::new(1)));
    let value = b.parameter(ManagedType::Int);
    let extra = b.parameter(ManagedType::Int);
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this, value],
    });
    b.value(
        ManagedType::Int,
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Super,
            result: Some(result),
            method: MethodRef::new(5),
            args: vec![this, value],
        },
        true,
    );
    b.push(ManagedInsn::Return { value: None });
    let ir = b.finish();
    let bindings = ParameterBindings {
        receiver: Some(this),
        parameters: vec![other, value, extra],
        names: vec![],
    };
    let r = good();
    let plan = plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &r).unwrap();
    assert!(plan.source.contains("super.select((int) (p1))"));
    let mut missing = good();
    missing.selected_super = false;
    assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &missing).is_err());
    let mut checked = good();
    checked.checked = true;
    assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &bindings, &checked).is_err());
}

fn home() -> PathBuf {
    std::env::var_os("DECOMPHIFY_JDK25_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into())
}
fn run(tool: &str, args: &[&str], dir: &std::path::Path) -> Vec<u8> {
    let output = Command::new(home().join("bin").join(tool))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
#[test]
fn source25_jvm_differential_effect_throw_and_oom_boundary() {
    let (ir, b) = body(0);
    let p = plan_constructor(&ir, MethodRef::new(0), "Base", &b, &good()).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "decomphify-source25-effects-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Fixture.java"),
        format!(
            "public class Fixture extends Base {{public final int captured;\n{}}}",
            p.source
        ),
    )
    .unwrap();
    std::fs::write(dir.join("Driver.java"),r#"class Probe {
 static String trace="";static volatile int shared=7;public int value=5;
 static void mark(int n){trace+=n==1?"A":"B";}
 static int combine(int a,int b){trace+="C";return a+b;}
 public int read(){trace+="R";return 3;}
}
class Base {Base(int n){Probe.trace+="P";if(((Fixture)this).captured!=n)throw new AssertionError("capture before parent");}}
public class Driver {
 public static void main(String[] args){
  for(int mode=0;mode<5;mode++){Probe.trace="";Probe.shared=7;
   try{new Fixture(mode==3?null:new Probe(),mode==1?-1:mode==4?Integer.MAX_VALUE:6,mode==2?0:2);System.out.println(Probe.trace+":"+Probe.shared);}
   catch(Throwable t){if(mode==4&&!(t instanceof OutOfMemoryError))throw new AssertionError("OOM oracle",t);System.out.println(Probe.trace+":"+Probe.shared+":"+t.getClass().getSimpleName());}
  }
 }
}"#).unwrap();
    run(
        "javac",
        &["--release", "25", "Fixture.java", "Driver.java"],
        &dir,
    );
    let after = run("java", &["-Xverify:all", "-Xmx16m", "Driver"], &dir);
    std::fs::write(dir.join("Original.java"),r#"import java.nio.file.*;import java.lang.classfile.*;import java.lang.constant.*;
public class Original {
 static final ClassDesc PROBE=ClassDesc.of("Probe"),SELF=ClassDesc.of("Fixture");
 public static void main(String[] args)throws Exception {
  ClassFile cf=ClassFile.of();ClassModel m=cf.parse(Path.of("Fixture.class"));
  byte[] bytes=cf.transformClass(m,(cb,e)->{
   if(e instanceof MethodModel mm&&mm.methodName().equalsString("<init>"))cb.transformMethod(mm,(mb,me)->{
    if(me instanceof CodeModel)mb.withCode(c->{
     c.iconst_1().invokestatic(PROBE,"mark",MethodTypeDesc.ofDescriptor("(I)V"));
     c.iload(2).newarray(TypeKind.BYTE).astore(4);c.aload(4).arraylength().istore(5);
     c.iload(5).iload(3).idiv().istore(6);
     c.getstatic(PROBE,"shared",ConstantDescs.CD_int).istore(7);
     c.iload(6).putstatic(PROBE,"shared",ConstantDescs.CD_int);
     c.aload(1).invokevirtual(PROBE,"read",MethodTypeDesc.ofDescriptor("()I")).istore(8);
     c.aload(1).getfield(PROBE,"value",ConstantDescs.CD_int).istore(9);
     c.iload(7).iload(8).iadd().iload(9).invokestatic(PROBE,"combine",MethodTypeDesc.ofDescriptor("(II)I")).istore(10);
     c.aload(0).iload(10).putfield(SELF,"captured",ConstantDescs.CD_int);
     c.iconst_2().invokestatic(PROBE,"mark",MethodTypeDesc.ofDescriptor("(I)V"));
     c.aload(0).iload(10).invokespecial(ClassDesc.of("Base"),"<init>",MethodTypeDesc.ofDescriptor("(I)V")).return_();
    });else mb.with(me);
   });else cb.with(e);
  });Files.write(Path.of("Fixture.class"),bytes);
 }
}"#).unwrap();
    run("javac", &["--release", "25", "Original.java"], &dir);
    run("java", &["Original"], &dir);
    let before = run("java", &["-Xverify:all", "-Xmx16m", "Driver"], &dir);
    assert_eq!(before, after);
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "ARCBP:3\nA:7:NegativeArraySizeException\nA:7:ArithmeticException\nA:3:NullPointerException\nA:7:OutOfMemoryError\n"
    );
}
