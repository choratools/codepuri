#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaMethod, JavaResolver, ParameterBindings, SourceResult, emit_method};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::Constant,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

fn field(owner: &str, name: &str, ty: JavaType) -> JavaField {
    JavaField {
        owner: owner.into(),
        name: name.into(),
        ty,
        is_static: true,
    }
}

#[test]
fn exact_owner_opt_in_and_instance_dispatch_negatives() {
    let mut target = field("f.m$type", "VALUE", JavaType::Int);
    assert_eq!(owner(&target, false).unwrap(), "f.m$type");
    assert_eq!(owner(&target, true).unwrap(), "((f.m$type) null)");
    target.is_static = false;
    assert!(owner(&target, true).is_err());
    target.is_static = true;
    target.ty = JavaType::Void;
    assert!(owner(&target, true).is_err());
    target.ty = JavaType::Int;
    target.owner = "f.m$type; altered".into();
    assert!(owner(&target, true).is_err());
}

#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(
        &mut self,
        ty: ManagedType,
        instruction: impl FnOnce(ValueId) -> ManagedInsn,
    ) -> ValueId {
        let mut result = None;
        let id = self.insns.insert_with_key(|id| {
            let value = self.values.insert(Value::new(id, ty));
            result = Some(value);
            instruction(value)
        });
        self.order.push(id);
        result.unwrap()
    }
    fn push(&mut self, instruction: ManagedInsn) {
        self.order.push(self.insns.insert(instruction));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for instruction in &self.order[1..] {
            let next = cfg.add_block(*instruction);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}

struct Legacy;
impl JavaResolver for Legacy {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(field("f.m$type", "VALUE", JavaType::Int))
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
struct Attested;
impl JavaResolver for Attested {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        Legacy.method(reference)
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        Legacy.field(reference)
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        Legacy.class(reference)
    }
    fn string(&self, reference: StringRef) -> Option<Vec<u16>> {
        Legacy.string(reference)
    }
    fn static_field_type_context(&self, reference: FieldRef, caller: &str) -> bool {
        reference.index() == 0 && caller == "shadow.Caller"
    }
}

fn emitted(write: bool, resolver: &impl JavaResolver) -> String {
    let mut builder = Builder::default();
    let value = if write {
        let value = builder.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(23),
        });
        builder.push(ManagedInsn::FieldPut {
            field: FieldRef::new(0),
            object: None,
            value,
        });
        None
    } else {
        Some(
            builder.value(ManagedType::Int, |result| ManagedInsn::FieldGet {
                result,
                field: FieldRef::new(0),
                object: None,
            }),
        )
    };
    builder.push(ManagedInsn::Return { value });
    let method = JavaMethod {
        owner: "shadow.Caller".into(),
        name: if write { "write" } else { "read" }.into(),
        is_static: true,
        return_type: if write { JavaType::Void } else { JavaType::Int },
        parameters: vec![],
        throws: vec![],
    };
    match emit_method(
        &builder.finish(),
        &method,
        &ParameterBindings::default(),
        resolver,
    ) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("static access unavailable: {other:?}"),
    }
}

#[test]
fn unchanged_backend_read_write_emission_uses_only_attested_qualifier() {
    assert!(!Legacy.static_field_type_context(FieldRef::new(0), "shadow.Caller"));
    assert!(!Attested.static_field_type_context(FieldRef::new(1), "shadow.Caller"));
    assert!(!Attested.static_field_type_context(FieldRef::new(0), "other.Caller"));
    for write in [false, true] {
        let old = emitted(write, &Legacy);
        let new = emitted(write, &Attested);
        assert!(old.contains("f.m$type.VALUE"));
        assert!(new.contains("((f.m$type) null).VALUE"));
        assert_eq!(
            new,
            old.replace("f.m$type.VALUE", "((f.m$type) null).VALUE")
        );
    }
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let temp = Self(std::env::temp_dir().join(format!(
            "static-field-qualifier-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir(&temp.0).unwrap();
        temp
    }
    fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, source).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tool(name: &str) -> PathBuf {
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let path = PathBuf::from(home).join("bin").join(name);
        if path.is_file() {
            return path;
        }
    }
    if Command::new(name).arg("-version").output().is_ok() {
        return name.into();
    }
    PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/bin").join(name)
}
fn compile(root: &Path, sources: &[PathBuf]) -> Output {
    Command::new(tool("javac"))
        .arg("-d")
        .arg(root)
        .args(sources)
        .output()
        .unwrap()
}
fn run(root: &Path, class: &str, arguments: &[&str]) -> Output {
    Command::new(tool("java"))
        .arg("-cp")
        .arg(root)
        .arg(class)
        .args(arguments)
        .output()
        .unwrap()
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn expression(owner_name: &str, name: &str, ty: JavaType) -> String {
    format!(
        "{}.{}",
        owner(&field(owner_name, name, ty), true).unwrap(),
        name
    )
}

#[test]
fn jvm_null_primary_preserves_initialization_order_writes_failures_and_identity() {
    let temp = Temp::new();
    let value = expression("f.m$type", "VALUE", JavaType::Int);
    let throwing = expression("f.Fail", "VALUE", JavaType::Int);
    let token = expression(
        "f.Refs",
        "TOKEN",
        JavaType::Class("java.lang.Object".into()),
    );
    let null = expression(
        "f.Refs",
        "EMPTY",
        JavaType::Class("java.lang.Object".into()),
    );
    let constant = expression("f.Constants", "CONST", JavaType::Int);
    let interface = expression("f.Fields", "VALUE", JavaType::Int);
    let sources = [
        temp.write("runtime/Trace.java", "package runtime; public class Trace { public static String log=\"\"; public static final Object token=new Object(); public static int mark(String s,int v){log+=s;return v;} public static boolean fail(){return true;} }"),
        temp.write("f/m$type.java", "package f; public class m$type { public static int VALUE=runtime.Trace.mark(\"M\",7); }"),
        temp.write("f/Fail.java", "package f; public class Fail { public static int VALUE; static {runtime.Trace.log+=\"F\";if(runtime.Trace.fail())throw new IllegalStateException(\"original\");} }"),
        temp.write("f/Refs.java", "package f; public class Refs { static {runtime.Trace.log+=\"O\";} public static final Object TOKEN=runtime.Trace.token; public static final Object EMPTY=null; }"),
        temp.write("f/Constants.java", "package f; public class Constants { static {runtime.Trace.log+=\"C\";} public static final int CONST=31; }"),
        temp.write("f/Fields.java", "package f; public interface Fields { int VALUE=runtime.Trace.mark(\"I\",9); static int method(){return 1;} }"),
        temp.write("shadow/Caller.java", &format!("package shadow; public class Caller {{ public final int f=17; public int read(){{runtime.Trace.log+=\"b\";int x={value};runtime.Trace.log+=\"a\";return x;}} public void write(){{runtime.Trace.log+=\"b\";{value}=rhs();runtime.Trace.log+=\"a\";}} static int rhs(){{return runtime.Trace.mark(\"R\",11);}} public int badRead(){{runtime.Trace.log+=\"b\";return {throwing};}} public void badWrite(){{runtime.Trace.log+=\"b\";{throwing}=rhs();}} public boolean refs(){{return {token}==runtime.Trace.token && {token}=={token} && {null}==null;}} public int constant(){{return {constant};}} public int iface(){{return {interface};}} }}")),
        temp.write("Driver.java", "public class Driver { public static void main(String[]args){ runtime.Trace.log=\"\";shadow.Caller c=new shadow.Caller();switch(args[0]){case \"read\":if(c.read()!=7)throw new AssertionError();break;case \"write\":c.write();if(c.read()!=11)throw new AssertionError();break;case \"badRead\":try{c.badRead();throw new AssertionError();}catch(ExceptionInInitializerError e){if(!(e.getCause() instanceof IllegalStateException))throw new AssertionError();runtime.Trace.log+=\"E\";}try{c.badRead();throw new AssertionError();}catch(NoClassDefFoundError e){runtime.Trace.log+=\"N\";}break;case \"badWrite\":try{c.badWrite();throw new AssertionError();}catch(ExceptionInInitializerError e){runtime.Trace.log+=\"E\";}try{c.badWrite();throw new AssertionError();}catch(NoClassDefFoundError e){runtime.Trace.log+=\"N\";}break;case \"refs\":if(!c.refs())throw new AssertionError();break;case \"constant\":if(c.constant()!=31)throw new AssertionError();break;case \"interface\":if(c.iface()!=9)throw new AssertionError();break;default:throw new AssertionError();}System.out.println(runtime.Trace.log);} }"),
    ];
    success(&compile(&temp.0, &sources));
    for (mode, expected) in [
        ("read", "bMa"),
        ("write", "bRMaba"),
        ("badRead", "bFEbN"),
        ("badWrite", "bRFEbRN"),
        ("refs", "O"),
        ("constant", ""),
        ("interface", "I"),
    ] {
        let output = run(&temp.0, "Driver", &[mode]);
        success(&output);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim_end(),
            expected,
            "{mode}"
        );
    }
    let bytecode = Command::new(tool("javap"))
        .args(["-c", "-p", "-classpath"])
        .arg(&temp.0)
        .arg("shadow.Caller")
        .output()
        .unwrap();
    success(&bytecode);
    let code = String::from_utf8_lossy(&bytecode.stdout);
    assert!(code.contains("getstatic") && code.contains("putstatic"));
    assert!(code.contains("Field f/m$type.VALUE:I"));
    assert!(
        !code.contains("getfield"),
        "null primary must never become an instance read: {code}"
    );
    let illegal_call = temp.write(
        "Bad.java",
        "class Bad {int f;int call(){return ((f.Fields)null).method();}}",
    );
    let illegal = Command::new(tool("javac"))
        .arg("-cp")
        .arg(&temp.0)
        .arg("-d")
        .arg(&temp.0)
        .arg(illegal_call)
        .output()
        .unwrap();
    assert!(!illegal.status.success());
    let diagnostic = String::from_utf8_lossy(&illegal.stderr);
    assert!(diagnostic.contains("static") && diagnostic.contains("interface"));
}

#[test]
fn null_checkcast_does_not_resolve_missing_owner_before_constant_or_write_rhs() {
    let temp = Temp::new();
    let constant = expression("f.Missing", "CONST", JavaType::Int);
    let value = expression("f.Missing", "VALUE", JavaType::Int);
    let sources = [
        temp.write("f/Missing.java", "package f; public class Missing { public static final int CONST=31; public static int VALUE; }"),
        temp.write("shadow/Caller.java", &format!("package shadow; public class Caller {{ int f; public int constant(){{return {constant};}} public void write(){{{value}=rhs();}} static int rhs(){{System.out.print(\"R\");return 3;}} }}")),
        temp.write("Original.java", "class Original { int constant(){return f.Missing.CONST;} void write(){f.Missing.VALUE=rhs();} static int rhs(){System.out.print(\"R\");return 3;} }"),
        temp.write("Driver.java", "public class Driver { public static void main(String[]a){try{if(a[0].equals(\"new\")){shadow.Caller c=new shadow.Caller();System.out.print(c.constant());c.write();}else{Original c=new Original();System.out.print(c.constant());c.write();}throw new AssertionError();}catch(NoClassDefFoundError e){System.out.println(\"N\");}} }"),
    ];
    success(&compile(&temp.0, &sources));
    std::fs::remove_file(temp.0.join("f/Missing.class")).unwrap();
    for mode in ["old", "new"] {
        let output = run(&temp.0, "Driver", &[mode]);
        success(&output);
        assert_eq!(String::from_utf8_lossy(&output.stdout), "31RN\n");
    }
}

#[test]
fn instance_and_altered_declaring_owner_are_not_equivalent_remedies() {
    let temp = Temp::new();
    let correct = expression("f.Owner", "VALUE", JavaType::Int);
    let changed = expression("f.Other", "VALUE", JavaType::Int);
    let sources = [
        temp.write("f/Owner.java", "package f; public class Owner {public static int VALUE=7;public int instance=11;}"),
        temp.write("f/Other.java", "package f; public class Other {public static int VALUE=9;}"),
        temp.write("Driver.java", &format!("public class Driver {{int f; int correct(){{return {correct};}}int changed(){{return {changed};}}int instance(){{return ((f.Owner)null).instance;}}public static void main(String[]a){{Driver d=new Driver();if(d.correct()!=7||d.changed()!=9)throw new AssertionError();try{{d.instance();throw new AssertionError();}}catch(NullPointerException expected){{}}}}}}")),
    ];
    success(&compile(&temp.0, &sources));
    success(&run(&temp.0, "Driver", &[]));
}

#[test]
fn exact_wave13_statement_shapes_fail_before_and_compile_with_unchanged_emission() {
    // Frozen original units remain outside this reduced field-qualifier proof:
    // cd/d$type.java pool23772 line37 and material/navigation/c$type.java
    // pool31950 lines6096/6098. Only their exact read statements and lexical
    // fields are reproduced here; no Android or whole-unit runtime claim.
    let temp = Temp::new();
    let resource = temp.write("f/m$type.java", "package f; public class m$type {public static final int[] TextAppearance=new int[]{3};public static final int TextAppearance_android_textSize=5;}" );
    let old_cd = "package cd; public class d$type {public final int f;public d$type(){int[] v1 = f.m$type.TextAppearance;this.f=v1[0];}}";
    let old_nav = "package com.google.android.material.navigation; public class c$type {public int f; public void u(){int[] v5;int v7;v5 = f.m$type.TextAppearance;v7 = f.m$type.TextAppearance_android_textSize;f=v5[0]+v7;}}";
    let cd = temp.write("cd/d$type.java", old_cd);
    let nav = temp.write(
        "com/google/android/material/navigation/c$type.java",
        old_nav,
    );
    let output = compile(&temp.0, &[resource.clone(), cd.clone(), nav.clone()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("int cannot be dereferenced"));
    let owner_name = owner(
        &field(
            "f.m$type",
            "TextAppearance",
            JavaType::Array(Box::new(JavaType::Int)),
        ),
        true,
    )
    .unwrap();
    std::fs::write(&cd, old_cd.replace("f.m$type.", &format!("{owner_name}."))).unwrap();
    std::fs::write(
        &nav,
        old_nav.replace("f.m$type.", &format!("{owner_name}.")),
    )
    .unwrap();
    let driver = temp.write("Driver.java", "public class Driver {public static void main(String[]a){cd.d$type d=new cd.d$type();com.google.android.material.navigation.c$type c=new com.google.android.material.navigation.c$type();c.u();if(d.f!=3||c.f!=8)throw new AssertionError();}}" );
    success(&compile(&temp.0, &[resource, cd, nav, driver]));
    success(&run(&temp.0, "Driver", &[]));
}
