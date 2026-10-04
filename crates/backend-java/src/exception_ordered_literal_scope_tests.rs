//! Separate original-order proof: resolving strings never become pure gaps.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::type_compatibility::{Declaration, Hierarchy};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    types::ManagedType,
    value::Value,
};

struct Md {
    region: super::super::super::scopes::RegionScope,
    original: OriginalScope,
    h: Hierarchy,
    catch: String,
    all: bool,
}
impl RegionMetadata for Md {
    fn region_scope(&self) -> &super::super::super::scopes::RegionScope {
        &self.region
    }
    fn original_scope(&self) -> Option<&OriginalScope> {
        Some(&self.original)
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.h
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class(self.catch.clone()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        self.all
    }
}
struct R {
    h: Hierarchy,
    missing_string: bool,
}
impl JavaResolver for R {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "OrderedLiteralBoundary.Item"
            } else {
                "java.lang.String"
            }
            .into(),
        )
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(if r.index() == 0 {
            JavaMethod {
                owner: "OrderedLiteralBoundary.Item".into(),
                name: "<init>".into(),
                is_static: false,
                parameters: vec![JavaType::Class("java.lang.String".into())],
                return_type: JavaType::Void,
                throws: vec![],
            }
        } else if r.index() == 2 {
            JavaMethod {
                owner: "OrderedLiteralBoundary".into(),
                name: "produce".into(),
                is_static: true,
                parameters: vec![],
                return_type: JavaType::Class("java.lang.String".into()),
                throws: vec![],
            }
        } else {
            JavaMethod {
                owner: "OrderedLiteralBoundary".into(),
                name: "observe".into(),
                is_static: true,
                parameters: vec![JavaType::Class("java.lang.String".into())],
                return_type: JavaType::Void,
                throws: vec![],
            }
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        (!self.missing_string).then(|| "yyyy-MM-dd kk:mm:ss.S z".encode_utf16().collect())
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        self.h.assignable(a, b) == Compatibility::Assignable
    }
}
fn hierarchy(unknown: bool) -> Hierarchy {
    let mut h = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        (
            "java.lang.NullPointerException",
            Some("java.lang.RuntimeException"),
        ),
        ("java.lang.String", Some("java.lang.Object")),
        ("OrderedLiteralBoundary.Item", Some("java.lang.Object")),
    ] {
        if unknown && name == "java.lang.Error" {
            continue;
        }
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
struct Fixture {
    ir: MethodIr<SsaForm>,
    md: Vec<Md>,
    r: R,
    chain: Vec<InsnId>,
    parameter: ValueId,
}
fn fixture(mode: u8) -> Fixture {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let seed = insns.insert(ManagedInsn::Return { value: None });
    let parameter = values.insert(Value::new(seed, ManagedType::Reference(ClassRef::new(1))));
    insns[seed] = ManagedInsn::Opaque {
        family_tag: 0x8003,
        reads: vec![],
        writes: vec![parameter],
        effects: EffectFlags::NONE,
    };
    let first = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(first);
    let blocks = (0..6)
        .map(|_| cfg.add_block(insns.insert(ManagedInsn::Return { value: None })))
        .collect::<Vec<_>>();
    let [alias, literal, ctor, normal, phi, handler] = blocks.as_slice() else {
        unreachable!()
    };
    let new = cfg.entry();
    let ids = cfg
        .block_ids()
        .map(|b| (b, *cfg.block(b).unwrap().payload()))
        .collect::<HashMap<_, _>>();
    let id = |b| ids[&b];
    let n = values.insert(Value::new(
        id(new),
        ManagedType::Reference(ClassRef::new(0)),
    ));
    let a = values.insert(Value::new(
        id(*alias),
        ManagedType::Reference(ClassRef::new(0)),
    ));
    let s = values.insert(Value::new(
        id(*literal),
        ManagedType::Reference(ClassRef::new(1)),
    ));
    let p = values.insert(Value::new(
        id(*phi),
        ManagedType::Reference(ClassRef::new(1)),
    ));
    insns[id(new)] = ManagedInsn::NewInstance {
        result: n,
        class: ClassRef::new(0),
    };
    insns[id(*alias)] = ManagedInsn::Move {
        result: a,
        value: n,
    };
    insns[id(*literal)] = if mode == 9 {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(s),
            method: MethodRef::new(2),
            args: vec![],
        }
    } else {
        ManagedInsn::Const {
            result: s,
            value: Constant::String(StringRef::new(0)),
        }
    };
    insns[id(*ctor)] = ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![a, s],
    };
    for (x, y) in [
        (new, *alias),
        (*alias, *literal),
        (*literal, *ctor),
        (*ctor, *normal),
        (*phi, *handler),
    ] {
        cfg.add_edge(x, y, EdgeKind::Unconditional).unwrap();
    }
    let left = cfg
        .add_edge(new, *phi, EdgeKind::Exception("typed".into()))
        .unwrap();
    let right = cfg
        .add_edge(*ctor, *phi, EdgeKind::Exception("typed".into()))
        .unwrap();
    insns[id(*phi)] = ManagedInsn::Phi {
        result: p,
        inputs: vec![
            (left, parameter),
            (right, if mode == 1 { s } else { parameter }),
        ],
    };
    insns[id(*handler)] = ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(1),
        args: vec![p],
    };
    cfg.add_edge(*handler, *normal, EdgeKind::Unconditional)
        .unwrap();
    let mut offsets = HashMap::from([
        (id(new), 10),
        (id(*literal), 12),
        (id(*ctor), 14),
        (id(*normal), 16),
        (id(*handler), 20),
    ]);
    if mode == 5 {
        offsets.remove(&id(*literal));
    }
    let h = hierarchy(mode == 4);
    let catch = match mode {
        2 => "java.lang.Error",
        3 => "java.lang.Throwable",
        _ => "java.lang.NullPointerException",
    }
    .to_owned();
    let md = [(0, 10, 12, new), (1, 14, 16, *ctor)]
        .into_iter()
        .map(|(ordinal, start, end, protected)| Md {
            region: super::super::super::scopes::RegionScope {
                protected: vec![protected],
                catches: vec![super::super::super::scopes::Catch {
                    key: "typed".into(),
                    entry: *phi,
                }],
                continuation: None,
            },
            original: OriginalScope {
                ordinal,
                start,
                end,
                handler_offset: ordinal as u16,
                catches: vec![("typed".into(), if mode == 7 { 21 } else { 20 })],
                offsets: offsets.clone(),
            },
            h: h.clone(),
            catch: catch.clone(),
            all: mode == 8,
        })
        .collect();
    let chain = vec![id(new), id(*alias), id(*literal), id(*ctor)];
    Fixture {
        ir: MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        md,
        r: R {
            h,
            missing_string: mode == 6,
        },
        chain,
        parameter,
    }
}
#[test]
fn resolving_literal_has_only_an_original_order_witness_and_immutable_input_transfer() {
    let f = fixture(0);
    let md =
        f.md.iter()
            .map(|m| m as &dyn RegionMetadata)
            .collect::<Vec<_>>();
    assert!(!super::super::cross_scope::prove(&f.ir, &md, &f.r).contains_key(&f.chain));
    assert!(
        prove(&f.ir, &md, &f.r)
            .get(&f.chain)
            .is_some_and(Witness::attested)
    );
    let proof = super::super::prove(&f.ir, &md, &f.r);
    assert!(!proof.pure.contains_key(&f.chain));
    assert!(!proof.chains.contains(&f.chain));
    let m = JavaMethod {
        owner: "OrderedLiteralBoundary".into(),
        name: "build".into(),
        is_static: true,
        parameters: vec![JavaType::Class("java.lang.String".into())],
        return_type: JavaType::Void,
        throws: vec![],
    };
    let bindings = crate::DeclarationBindings {
        parameters: vec![Some(f.parameter)],
        ..Default::default()
    };
    let result = super::super::super::dispatch::emit_method(&f.ir, &m, &bindings, &f.r, &md);
    let crate::SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    assert!(source.contains("new OrderedLiteralBoundary.Item("));
    assert!(source.contains("original allocation argument sequence"));
    assert!(!source.contains(".intern()"));
}
#[test]
fn string_error_catches_unknown_domain_missing_origin_and_changed_prethrow_input_deny() {
    for mode in 1..=9 {
        let f = fixture(mode);
        let md =
            f.md.iter()
                .map(|m| m as &dyn RegionMetadata)
                .collect::<Vec<_>>();
        assert!(
            !prove(&f.ir, &md, &f.r).contains_key(&f.chain),
            "mode {mode}"
        );
    }
}
