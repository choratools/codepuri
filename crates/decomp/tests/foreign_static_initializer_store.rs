//! Explicit DEX stores and runtime capability evidence are independent contracts.
#![allow(clippy::unwrap_used)]
use decomp::java_class_kinds::initializers::*;
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, ClassDef, EncodedField, EncodedMethod},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn string(dex: &mut DexFile, text: &str) -> u32 {
    let mut s = dex.strings[0].clone();
    s.value = text.into();
    let index = dex.strings.len() as u32;
    dex.strings.push(s);
    index
}
fn ty(dex: &mut DexFile, text: &str) -> u32 {
    let descriptor_idx = string(dex, text);
    let index = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}
fn fixture(flags: u32) -> (DexFile, ClassDef) {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let mut class = dex.classes[0].clone();
    class.class_idx = ty(&mut dex, "LFixture;");
    class.access_flags = flags;
    class.superclass_idx = ty(&mut dex, "Ljava/lang/Object;");
    class.interfaces = None;
    class.annotations = None;
    class.static_values = None;
    class.class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![],
        virtual_methods: vec![],
        span: SPAN,
    });
    (dex, class)
}
fn element(dex: &mut DexFile, class: &mut ClassDef, name: &str, descriptor: &str) -> EncodedMethod {
    let return_type_idx = ty(dex, descriptor);
    let proto_idx = dex.proto_ids.len() as u16;
    dex.proto_ids.push(ProtoId {
        shorty_idx: 0,
        return_type_idx,
        parameters_off: 0,
        parameters: None,
        span: SPAN,
    });
    let name_idx = string(dex, name);
    let method_idx = dex.method_ids.len() as u32;
    dex.method_ids.push(MethodId {
        class_idx: class.class_idx as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    let method = EncodedMethod {
        method_idx,
        access_flags: 0x401,
        code_off: 0,
        code: None,
    };
    class
        .class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .push(method.clone());
    method
}
fn field(dex: &mut DexFile, class: &mut ClassDef, name: &str, descriptor: &str, flags: u32) -> u32 {
    let type_idx = ty(dex, descriptor) as u16;
    let name_idx = string(dex, name);
    let index = dex.field_ids.len() as u32;
    dex.field_ids.push(FieldId {
        class_idx: class.class_idx as u16,
        type_idx,
        name_idx,
        span: SPAN,
    });
    let f = EncodedField {
        field_idx: index,
        access_flags: flags,
    };
    if flags & 8 != 0 {
        class.class_data.as_mut().unwrap().static_fields.push(f);
    } else {
        class.class_data.as_mut().unwrap().instance_fields.push(f);
    }
    index
}
fn code(units: &[u16]) -> dex_format::code::CodeItem {
    dex_format::code::CodeItem {
        registers_size: 4,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: SPAN,
    }
}

use backend_java::constructors::{
    ConstructorClass, ConstructorResolver, Initializer, TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
struct Resolver<'a> {
    dex: &'a DexFile,
    known: bool,
}
impl JavaResolver for Resolver<'_> {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        decomp::dex_java::DexJavaResolver { dex: self.dex }.method(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        decomp::dex_java::DexJavaResolver { dex: self.dex }.field(r)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        decomp::dex_java::DexJavaResolver { dex: self.dex }.class(r)
    }
    fn field_access(&self, r: FieldRef) -> Option<u32> {
        if !self.known {
            return None;
        }
        self.dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.static_fields.iter().chain(&d.instance_fields))
            .find(|f| f.field_idx == r.index())
            .map(|f| f.access_flags)
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for Resolver<'_> {
    fn initializer(&self, _: MethodRef) -> Option<Initializer> {
        None
    }
}
impl TrustedConstructorResolver for Resolver<'_> {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: name.into(),
            superclass: match name {
                "Fixture" => Some("java.lang.Object".into()),
                "java.lang.Object" => None,
                _ => return None,
            },
            access_flags: if name == "Fixture" { 0x401 } else { 1 },
        })
    }
    fn constructor_access(&self, _: MethodRef) -> Option<u32> {
        None
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        if !self.known || owner != "Fixture" {
            return None;
        }
        Some(
            self.dex.classes[0]
                .class_data
                .as_ref()?
                .direct_methods
                .iter()
                .filter(|m| {
                    self.dex
                        .method_id(m.method_idx)
                        .ok()
                        .and_then(|id| self.dex.string(id.name_idx).ok())
                        == Some("<init>")
                })
                .map(|m| MethodRef::new(m.method_idx))
                .collect(),
        )
    }
}
fn input() -> (DexFile, u32, u32) {
    let (mut dex, mut class) = fixture(0x401);
    let f = field(&mut dex, &mut class, "N", "I", 0x19);
    let mut m = element(&mut dex, &mut class, "<clinit>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    m.access_flags = 0x10008;
    m.code_off = 1;
    let mut c = code(&[0x7012, 0x0067, f as u16, 0x000e]);
    c.registers_size = 1;
    c.ins_size = 0;
    m.code = Some(c);
    let idx = m.method_idx;
    class.class_data.as_mut().unwrap().direct_methods.push(m);
    dex.classes = vec![class];
    (dex, idx, f)
}

#[test]
fn foreign_public_nonfinal_requires_genuine_public_owner_and_flags() {
    for (owner_flags, field_flags, accepted) in [
        (1, 9, true),
        (0, 9, false),
        (1, 0x19, false),
        (1, 0xa, false),
        (1, 1, false),
    ] {
        let (mut dex, idx, _) = input();
        dex.classes[0]
            .class_data
            .as_mut()
            .unwrap()
            .static_fields
            .clear();
        let mut foreign = dex.classes[0].clone();
        foreign.class_idx = ty(&mut dex, "LForeign;");
        foreign.access_flags = owner_flags;
        foreign.class_data = Some(ClassData {
            static_fields: vec![],
            instance_fields: vec![],
            direct_methods: vec![],
            virtual_methods: vec![],
            span: SPAN,
        });
        let target = field(&mut dex, &mut foreign, "value", "I", field_flags);
        let original = &mut dex.classes[0].class_data.as_mut().unwrap().direct_methods[0];
        original.code = Some(code(&[0x7012, 0x0067, target as u16, 0x000e]));
        dex.classes.push(foreign);
        let inputs = [dex];
        let class = &inputs[0].classes[0];
        let m = &class.class_data.as_ref().unwrap().direct_methods[0];
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            m.code.as_ref().unwrap(),
            &dex_decompiler::DexTypes::new(&inputs[0], Some(idx)),
        )
        .unwrap();
        let inventory = write_inventory::WriteInventory::scan(&inputs);
        let r = Resolver {
            dex: &inputs[0],
            known: true,
        };
        let result = plan_static_initializer_explicit_stores_attested(
            &inputs,
            0,
            class,
            MethodRef::new(idx),
            &ssa,
            &r,
            &inventory,
        );
        assert_eq!(
            result.is_ok(),
            accepted,
            "owner={owner_flags:x} field={field_flags:x}: {:?}",
            result.as_ref().err()
        );
        if let Ok(result) = result {
            let unknown = Resolver {
                dex: &inputs[0],
                known: false,
            };
            assert!(
                plan_static_initializer_explicit_stores_attested(
                    &inputs,
                    0,
                    class,
                    MethodRef::new(idx),
                    &ssa,
                    &unknown,
                    &inventory
                )
                .is_err()
            );
            let body = result.plan.source;
            assert!(body.contains("Foreign.value ="));
            let dir = std::env::temp_dir()
                .join(format!("decomphify-foreign-store-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let source = format!(
                "abstract class Fixture {{{body}}} class State {{ static String log=\"\";static boolean fail;static RuntimeException original=new RuntimeException(\"foreign\");}} class Foreign {{public static int value;static {{State.log+=\"foreign\";if(State.fail)throw State.original;}}}} class Run {{public static void main(String[]args)throws Exception{{State.fail=args.length!=0;try{{Class.forName(\"Fixture\");if(State.fail||Foreign.value!=7)throw new AssertionError();}}catch(ExceptionInInitializerError e){{if(!State.fail||e.getCause()!=State.original)throw new AssertionError(e);}}if(!State.log.equals(\"foreign\"))throw new AssertionError(State.log);System.out.println(\"foreign-order-and-identity=ok\");}}}}"
            );
            std::fs::write(dir.join("Run.java"), source).unwrap();
            for home in [
                "/home/jtjisgod/tools/jdk-17.0.13+11",
                "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
            ] {
                for (tool, args) in [
                    ("javac", vec!["--release", "17", "Run.java"]),
                    ("java", vec!["Run"]),
                    ("java", vec!["Run", "fail"]),
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
                std::fs::write(dir.join("Fixture.class"), original_foreign_store_class()).unwrap();
                for args in [vec!["Run"], vec!["Run", "fail"]] {
                    let output = std::process::Command::new(format!("{home}/bin/java"))
                        .args(args)
                        .current_dir(&dir)
                        .output()
                        .unwrap();
                    assert!(
                        output.status.success(),
                        "original bytecode: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert_eq!(
                        String::from_utf8_lossy(&output.stdout).trim(),
                        "foreign-order-and-identity=ok"
                    );
                }
            }
        }
    }
}

// Original JVM analogue: no source constructor and exact bipush/putstatic/return.
fn original_foreign_store_class() -> Vec<u8> {
    fn u2(out: &mut Vec<u8>, value: u16) {
        out.extend(value.to_be_bytes());
    }
    fn u4(out: &mut Vec<u8>, value: u32) {
        out.extend(value.to_be_bytes());
    }
    fn utf(out: &mut Vec<u8>, text: &str) {
        out.push(1);
        u2(out, text.len() as u16);
        out.extend(text.as_bytes());
    }
    let mut out = vec![];
    u4(&mut out, 0xcafebabe);
    u2(&mut out, 0);
    u2(&mut out, 61);
    u2(&mut out, 14);
    utf(&mut out, "Fixture");
    out.push(7);
    u2(&mut out, 1);
    utf(&mut out, "java/lang/Object");
    out.push(7);
    u2(&mut out, 3);
    utf(&mut out, "Foreign");
    out.push(7);
    u2(&mut out, 5);
    utf(&mut out, "value");
    utf(&mut out, "I");
    out.push(12);
    u2(&mut out, 7);
    u2(&mut out, 8);
    out.push(9);
    u2(&mut out, 6);
    u2(&mut out, 9);
    utf(&mut out, "<clinit>");
    utf(&mut out, "()V");
    utf(&mut out, "Code");
    for value in [0x420, 2, 4, 0, 0, 1, 8, 11, 12, 1, 13] {
        u2(&mut out, value);
    }
    u4(&mut out, 18);
    u2(&mut out, 1);
    u2(&mut out, 0);
    u4(&mut out, 6);
    out.extend([0x10, 7, 0xb3, 0, 10, 0xb1]);
    u2(&mut out, 0);
    u2(&mut out, 0);
    u2(&mut out, 0);
    out
}
