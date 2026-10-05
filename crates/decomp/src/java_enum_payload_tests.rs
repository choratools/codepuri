//! Focused original-DEX and payload runtime proof; no APK-wide emission.
#![allow(clippy::unwrap_used)]
use super::*;
use backend_java::{JavaField, JavaMethod, JavaResolver};
use decomp_core::cfg::Cfg;
use dex_format::{
    RawSpan,
    class::{ClassData, EncodedField, EncodedMethod},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    refs::{ClassRef, FieldRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, Value as IrValue},
};
use std::process::Command;
struct Resolver<'a>(&'a DexFile);
impl JavaResolver for Resolver<'_> {
    fn class(&self, r: ClassRef) -> Option<String> {
        crate::dex_java::DexJavaResolver { dex: self.0 }.class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Some(self.0.string(r.index()).ok()?.encode_utf16().collect())
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        crate::dex_java::DexJavaResolver { dex: self.0 }.field(r)
    }
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        let m = self.0.method_id(reference.index()).ok()?;
        let p = self.0.proto_id(m.proto_idx.into()).ok()?;
        let type_at = |i: u32| {
            crate::dex_java::descriptor_type(
                self.0.string(self.0.type_id(i).ok()?.descriptor_idx).ok()?,
            )
        };
        let JavaType::Class(owner) = type_at(m.class_idx.into())? else {
            return None;
        };
        Some(JavaMethod {
            owner,
            name: self.0.string(m.name_idx).ok()?.into(),
            is_static: self
                .0
                .classes
                .iter()
                .filter_map(|c| c.class_data.as_ref())
                .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
                .find(|m| m.method_idx == reference.index())
                .is_some_and(|m| m.access_flags & 8 != 0),
            return_type: type_at(p.return_type_idx)?,
            parameters: p
                .parameters
                .as_ref()
                .map(|p| {
                    p.types
                        .iter()
                        .map(|i| type_at(u32::from(*i)))
                        .collect::<Option<Vec<_>>>()
                })
                .unwrap_or(Some(vec![]))?,
            throws: vec![],
        })
    }
}
impl backend_java::constructors::ConstructorResolver for Resolver<'_> {
    fn initializer(&self, r: MethodRef) -> Option<backend_java::constructors::Initializer> {
        let method = self.method(r)?;
        if method.name != "<init>" {
            return None;
        }
        Some(backend_java::constructors::Initializer {
            method,
            delegates_to: None,
        })
    }
}
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
#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, ty: ManagedType, make: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut result = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(IrValue::new(id, ty));
            result = Some(v);
            make(v)
        });
        self.order.push(id);
        result.unwrap()
    }
    fn parameter(&mut self) -> ValueId {
        let v = self.value(ManagedType::Unknown, |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        });
        self.order.pop();
        v
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

fn bindings(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    code: &dex_format::code::CodeItem,
) -> DeclarationBindings {
    let p = crate::java_initial_parameters::parameter_bindings(
        ir,
        method,
        code.registers_size,
        code.ins_size,
    )
    .unwrap();
    let bound = |p| match p {
        crate::java_initial_parameters::InitialParameter::Bound(v) => Some(v),
        crate::java_initial_parameters::InitialParameter::Unused { .. } => None,
    };
    DeclarationBindings {
        receiver: p.receiver.and_then(bound),
        parameters: p.parameters.into_iter().map(bound).collect(),
        names: vec![],
    }
}

#[test]
#[ignore = "local Samsung Pass APK; only three original enum owners, no full export"]
fn samsung_original_enum_payload_method_evidence() {
    let apk = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    );
    let bytes = Command::new("unzip")
        .args(["-p", apk, "classes.dex"])
        .output()
        .unwrap();
    assert!(bytes.status.success());
    let dex = DexFile::from_bytes(&bytes.stdout).unwrap();
    let resolver = Resolver(&dex);
    for wanted in [
        "Landroidx/appcompat/widget/SearchView$q;",
        "Lcom/bumptech/glide/load/ImageHeaderParser$ImageType;",
        "Lcom/google/firebase/remoteconfig/internal/c$b;",
    ] {
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(&dex, c.class_idx).ok() == Some(wanted))
            .unwrap();
        let data = class.class_data.as_ref().unwrap();
        let constructors: Vec<_> = data
            .direct_methods
            .iter()
            .filter(|m| {
                dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "<init>"
            })
            .collect();
        let lift = |m: &EncodedMethod| {
            dex_decompiler::passes::DexPasses::run_ssa_with(
                m.code.as_ref().unwrap(),
                &dex_decompiler::DexTypes::new(&dex, Some(m.method_idx)),
            )
            .unwrap()
        };
        let ssas: Vec<_> = constructors.iter().map(|m| lift(m)).collect();
        let parameters: Vec<_> = constructors
            .iter()
            .zip(&ssas)
            .map(|(m, s)| {
                bindings(
                    s.ir(),
                    &resolver.method(MethodRef::new(m.method_idx)).unwrap(),
                    m.code.as_ref().unwrap(),
                )
            })
            .collect();
        let inputs: Vec<_> = constructors
            .iter()
            .zip(&ssas)
            .zip(&parameters)
            .map(|((m, s), b)| EnumConstructorDeclarationInput {
                reference: MethodRef::new(m.method_idx),
                ir: s.ir(),
                bindings: b,
            })
            .collect();
        let clinit = data
            .direct_methods
            .iter()
            .find(|m| {
                dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "<clinit>"
            })
            .unwrap();
        let clinit_ssa = lift(clinit);
        // R8 made these constructors public. Their body proof is independent
        // of whether Java enum declarations can preserve the original access.
        for input in &inputs {
            let body = plan_constructor_declaration(
                input.ir,
                input.reference,
                "java.lang.Enum",
                input.bindings,
                &resolver,
            )
            .unwrap();
            assert_eq!(body.chain, ChainKind::Super);
            assert!(!body.body.is_empty());
        }
        assert!(constructors.iter().all(|m| m.access_flags & 1 != 0));
        assert!(matches!(
            plan_enum_initialization_family(
                &dex,
                class,
                &inputs,
                MethodRef::new(clinit.method_idx),
                clinit_ssa.ir(),
                &resolver
            ),
            Err(Error::EnumConstructorAccessUnrepresentable { owner, method, access_flags })
                if owner == wanted && constructors.iter().any(|c|
                    c.method_idx == method && c.access_flags == access_flags)
        ));
        // Inventory every original method, including generated methods and all
        // user bodies; residual declarations must not count as emitted bodies.
        let identities: HashSet<_> = data
            .direct_methods
            .iter()
            .chain(&data.virtual_methods)
            .map(|m| m.method_idx)
            .collect();
        assert_eq!(
            identities.len(),
            data.direct_methods.len() + data.virtual_methods.len()
        );
        eprintln!(
            "{wanted}: public_constructor_unrepresentable; constructor bodies proved={:?}; retained original method identities={identities:?}",
            constructors
                .iter()
                .map(|m| m.method_idx)
                .collect::<Vec<_>>()
        );
    }
}

fn method(
    dex: &mut DexFile,
    class: &mut ClassDef,
    name: &str,
    parameters: &[&str],
    ret: &str,
    flags: u32,
) -> MethodRef {
    let m = element(dex, class, name, ret);
    class.class_data.as_mut().unwrap().virtual_methods.pop();
    let types: Vec<_> = parameters.iter().map(|p| ty(dex, p) as u16).collect();
    let proto = dex.method_ids[m.method_idx as usize].proto_idx as usize;
    dex.proto_ids[proto].parameters = Some(dex_format::ids::TypeList { types, span: SPAN });
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(EncodedMethod {
            access_flags: flags,
            ..m.clone()
        });
    MethodRef::new(m.method_idx)
}
fn literal(b: &mut Builder, value: Constant) -> ValueId {
    let ty = match value {
        Constant::Long(_) => ManagedType::Long,
        Constant::FloatBits(_) => ManagedType::Float,
        Constant::DoubleBits(_) => ManagedType::Double,
        Constant::String(_) | Constant::Null => ManagedType::Unknown,
        _ => ManagedType::Int,
    };
    b.value(ty, |result| ManagedInsn::Const { result, value })
}
struct Delegating<'a> {
    dex: &'a DexFile,
    from: MethodRef,
    to: MethodRef,
}
impl JavaResolver for Delegating<'_> {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Resolver(self.dex).method(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        Resolver(self.dex).field(r)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Resolver(self.dex).class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Resolver(self.dex).string(r)
    }
}
impl ConstructorResolver for Delegating<'_> {
    fn initializer(&self, r: MethodRef) -> Option<backend_java::constructors::Initializer> {
        let mut m = Resolver(self.dex).initializer(r)?;
        if r == self.from {
            m.delegates_to = Some(self.to);
        }
        Some(m)
    }
}
fn raw_code(words: &[u16], incoming: u16) -> dex_format::code::CodeItem {
    dex_format::code::CodeItem {
        registers_size: 2,
        ins_size: incoming,
        outs_size: 2,
        debug_info_off: 0,
        insns_size: words.len() as u32,
        insns: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: SPAN,
    }
}

#[test]
fn private_enum_payload_family_overloads_unused_null_bits_runtime() {
    let (mut dex, mut class) = fixture(0x4011);
    let enum_type = ty(&mut dex, "Ljava/lang/Enum;");
    class.superclass_idx = enum_type;
    let full = method(
        &mut dex,
        &mut class,
        "<init>",
        &[
            "Ljava/lang/String;",
            "I",
            "I",
            "Ljava/lang/String;",
            "Z",
            "J",
            "F",
            "D",
            "Ljava/lang/Object;",
        ],
        "V",
        0x10002,
    );
    let short = method(
        &mut dex,
        &mut class,
        "<init>",
        &["Ljava/lang/String;", "I", "I"],
        "V",
        0x10002,
    );
    let clinit = method(&mut dex, &mut class, "<clinit>", &[], "V", 0x10008);
    let mut base = class.clone();
    base.class_idx = enum_type;
    let enum_ctor = method(
        &mut dex,
        &mut base,
        "<init>",
        &["Ljava/lang/String;", "I"],
        "V",
        0x10004,
    );
    let mut fields = BTreeMap::new();
    let mut ids = Vec::new();
    for (name, desc) in [
        ("n", "I"),
        ("text", "Ljava/lang/String;"),
        ("enabled", "Z"),
        ("wide", "J"),
        ("single", "F"),
        ("doubleValue", "D"),
    ] {
        let id = field(&mut dex, &mut class, name, desc, 0x10);
        let java = crate::dex_java::descriptor_type(desc).unwrap();
        fields.insert(id, format!("final {} {name};", type_source(&java).unwrap()));
        ids.push(id);
    }
    let a = field(&mut dex, &mut class, "A", "LFixture;", 0x4019);
    let b = field(&mut dex, &mut class, "B", "LFixture;", 0x4019);
    let backing = field(&mut dex, &mut class, "$VALUES", "[LFixture;", 0x101a);
    let array = u32::from(dex.field_ids[backing as usize].type_idx);
    let values = method(&mut dex, &mut class, "values", &[], "[LFixture;", 9);
    let value_of = method(
        &mut dex,
        &mut class,
        "valueOf",
        &["Ljava/lang/String;"],
        "LFixture;",
        9,
    );
    let mut array_class = class.clone();
    array_class.class_idx = array;
    let clone = method(
        &mut dex,
        &mut array_class,
        "clone",
        &[],
        "Ljava/lang/Object;",
        1,
    );
    let enum_value_of = method(
        &mut dex,
        &mut base,
        "valueOf",
        &["Ljava/lang/Class;", "Ljava/lang/String;"],
        "Ljava/lang/Enum;",
        9,
    );
    let class_idx = class.class_idx as u16;
    for (reference, code) in [
        (
            values,
            raw_code(
                &[
                    0x0062,
                    backing as u16,
                    0x106e,
                    clone.index() as u16,
                    0,
                    0x000c,
                    0x001f,
                    array as u16,
                    0x0011,
                ],
                0,
            ),
        ),
        (
            value_of,
            raw_code(
                &[
                    0x001c,
                    class_idx,
                    0x2071,
                    enum_value_of.index() as u16,
                    0x0010,
                    0x000c,
                    0x001f,
                    class_idx,
                    0x0011,
                ],
                1,
            ),
        ),
    ] {
        let m = class
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|m| m.method_idx == reference.index())
            .unwrap();
        m.code = Some(code);
        m.code_off = 1;
    }
    dex.classes = vec![class.clone()];
    let mut full_body = Builder::default();
    let receiver = full_body.parameter();
    let name = full_body.parameter();
    let ordinal = full_body.parameter();
    let payload: Vec<_> = ids.iter().map(|_| full_body.parameter()).collect();
    full_body.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: enum_ctor,
        args: vec![receiver, name, ordinal],
    });
    for (field, value) in ids.iter().zip(&payload) {
        full_body.push(ManagedInsn::FieldPut {
            object: Some(receiver),
            field: FieldRef::new(*field),
            value: *value,
        });
    }
    full_body.push(ManagedInsn::Return { value: None });
    let full_bindings = DeclarationBindings {
        receiver: Some(receiver),
        parameters: [Some(name), Some(ordinal)]
            .into_iter()
            .chain(payload.into_iter().map(Some))
            .chain([None])
            .collect(),
        names: vec![],
    };
    let full_body = full_body.finish();
    let mut short_body = Builder::default();
    let receiver = short_body.parameter();
    let name = short_body.parameter();
    let ordinal = short_body.parameter();
    let n = short_body.parameter();
    let extras: Vec<_> = [
        Constant::Null,
        Constant::Int(1),
        Constant::Long(-23),
        Constant::FloatBits(0x80000000),
        Constant::DoubleBits(0x8000000000000000),
        Constant::Null,
    ]
    .into_iter()
    .map(|c| literal(&mut short_body, c))
    .collect();
    short_body.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: full,
        args: [receiver, name, ordinal, n]
            .into_iter()
            .chain(extras)
            .collect(),
    });
    short_body.push(ManagedInsn::Return { value: None });
    let short_bindings = DeclarationBindings {
        receiver: Some(receiver),
        parameters: vec![Some(name), Some(ordinal), Some(n)],
        names: vec![],
    };
    let short_body = short_body.finish();
    let mut init = Builder::default();
    let mut objects = Vec::new();
    for (ordinal, (field, label, ctor)) in [(a, "A", full), (b, "B", short)].into_iter().enumerate()
    {
        let object = init.value(
            ManagedType::Reference(ClassRef::new(class.class_idx)),
            |result| ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(class.class_idx),
            },
        );
        let name_idx = string(&mut dex, label);
        let name = literal(&mut init, Constant::String(StringRef::new(name_idx)));
        let ord = literal(&mut init, Constant::Int(ordinal as i32));
        let n = literal(
            &mut init,
            Constant::Int(if ordinal == 0 { -13 } else { 99 }),
        );
        let mut args = vec![object, name, ord, n];
        if ctor == full {
            let text = string(&mut dex, "slash\\u000a\"end");
            for c in [
                Constant::String(StringRef::new(text)),
                Constant::Int(0),
                Constant::Long(i64::MIN),
                Constant::FloatBits(0x7fc00001),
                Constant::DoubleBits(0x7ff8000000000001),
                Constant::Null,
            ] {
                args.push(literal(&mut init, c));
            }
        }
        init.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: ctor,
            args,
        });
        init.push(ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(field),
            value: object,
        });
        objects.push(object);
    }
    let length = literal(&mut init, Constant::Int(2));
    let filled = init.value(ManagedType::Unknown, |result| ManagedInsn::NewArray {
        result,
        element: TypeRef::new(array),
        length,
    });
    for (index, value) in objects.into_iter().enumerate() {
        let index = literal(&mut init, Constant::Int(index as i32));
        init.push(ManagedInsn::ArrayPut {
            array: filled,
            index,
            value,
        });
    }
    init.push(ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(backing),
        value: filled,
    });
    init.push(ManagedInsn::Return { value: None });
    let init = init.finish();
    let resolver = Delegating {
        dex: &dex,
        from: short,
        to: full,
    };
    let inputs = [
        EnumConstructorDeclarationInput {
            reference: full,
            ir: &full_body,
            bindings: &full_bindings,
        },
        EnumConstructorDeclarationInput {
            reference: short,
            ir: &short_body,
            bindings: &short_bindings,
        },
    ];
    for access_flags in [0x10001, 0x10004, 0x10000] {
        let mut changed = class.clone();
        let constructor = changed
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|method| method.method_idx == full.index())
            .unwrap();
        constructor.access_flags = access_flags;
        assert!(matches!(
            plan_enum_initialization_family(&dex, &changed, &inputs, clinit, &init, &resolver),
            Err(Error::EnumConstructorAccessUnrepresentable { owner, method, access_flags: flags })
                if owner == "LFixture;" && method == full.index() && flags == access_flags
        ));
    }
    let plan =
        plan_enum_initialization_family(&dex, &class, &inputs, clinit, &init, &resolver).unwrap();
    let methods = emission::plan_enum_methods(&dex, &class, &plan).unwrap();
    let source =
        emission::emit_enum_declaration(&class, &plan, &methods, &fields, &BTreeMap::new())
            .unwrap();
    assert!(source.contains("private Fixture(int p2)"));
    assert!(source.contains("java.lang.Object p8"));
    assert_eq!(plan.constructor_sources.len(), 2);
    assert_eq!(plan.constants[0].arguments.len(), 7);
    assert!(matches!(
        plan_enum_initialization_family(&dex, &class, &inputs[..1], clinit, &init, &resolver),
        Err(Error::EnumConstructorsUnavailable)
    ));
    assert!(
        emission::emit_enum_declaration(
            &class,
            &plan,
            &methods,
            &BTreeMap::new(),
            &BTreeMap::new()
        )
        .is_err()
    );
    let mut forbidden = Builder::default();
    let value = literal(&mut forbidden, Constant::Null);
    forbidden.push(ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(a),
        value,
    });
    forbidden.push(ManagedInsn::Return { value: None });
    assert!(emission::validate_enum_member_ir(&class, &plan, &forbidden.finish()).is_err());
    let dir = std::env::temp_dir().join(format!("decomphify-enum-payload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), &source).unwrap();
    std::fs::write(dir.join("EnumCheck.java"),r#"public class EnumCheck { public static void main(String[] args) {
        if (Fixture.values().length != 2 || Fixture.valueOf("B") != Fixture.B || !Fixture.A.name().equals("A") || Fixture.B.ordinal() != 1) throw new AssertionError();
        if(Fixture.A.n != -13 || !Fixture.A.text.equals("slash\\u000a\"end") || Fixture.A.enabled || Fixture.A.wide != Long.MIN_VALUE || Float.floatToRawIntBits(Fixture.A.single) != 0x7fc00001 || Double.doubleToRawLongBits(Fixture.A.doubleValue) != 0x7ff8000000000001L) throw new AssertionError();
        if(Fixture.B.n != 99 || Fixture.B.text != null || !Fixture.B.enabled || Fixture.B.wide != -23L || Float.floatToRawIntBits(Fixture.B.single) != 0x80000000 || Double.doubleToRawLongBits(Fixture.B.doubleValue) != 0x8000000000000000L) throw new AssertionError();
        Fixture[] clone = Fixture.values(); clone[0] = Fixture.B; if(Fixture.values()[0] != Fixture.A) throw new AssertionError();
    }}"#).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (binary, args) in [
        (
            "javac",
            vec!["--release", "17", "Fixture.java", "EnumCheck.java"],
        ),
        ("java", vec!["-cp", ".", "EnumCheck"]),
    ] {
        let output = Command::new(format!("{home}/bin/{binary}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{binary}: {}\n{source}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn enum_values_helper_proves_large_ordered_array_and_rejects_extra_allocation() {
    let (mut dex, mut class) = fixture(0x4011);
    let mut constants = Vec::new();
    for ordinal in 0..6 {
        let name = format!("C{ordinal}");
        let field_idx = field(&mut dex, &mut class, &name, "LFixture;", 0x4019);
        constants.push(EnumConstant {
            field_idx,
            name,
            ordinal,
            arguments: vec![],
        });
    }
    let helper = method(&mut dex, &mut class, "$values", &[], "[LFixture;", 0x100a);
    let array = ty(&mut dex, "[LFixture;") as u16;
    let mut words = vec![0x6112, 0x1023, array];
    for constant in &constants {
        words.extend([
            0x0262,
            constant.field_idx as u16,
            ((constant.ordinal as u16) << 12) | 0x0112,
            0x024d,
            0x0100,
        ]);
    }
    words.push(0x0011);
    let set_code = |class: &mut ClassDef, words: &[u16]| {
        let mut code = raw_code(words, 0);
        code.registers_size = 3;
        let m = class
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|m| m.method_idx == helper.index())
            .unwrap();
        m.code = Some(code);
        m.code_off = 1;
    };
    set_code(&mut class, &words);
    dex.classes = vec![class.clone()];
    assert!(emission::prove_values_helper(
        &dex,
        &class,
        helper.index(),
        &constants
    ));
    let mut duplicate = words.clone();
    duplicate.splice(3..3, [0x1023, array]);
    set_code(&mut class, &duplicate);
    assert!(!emission::prove_values_helper(
        &dex,
        &class,
        helper.index(),
        &constants
    ));
    let mut wrong_order = words.clone();
    wrong_order[5] = 0x1112;
    set_code(&mut class, &wrong_order);
    assert!(!emission::prove_values_helper(
        &dex,
        &class,
        helper.index(),
        &constants
    ));
}
