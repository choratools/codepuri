//! Compatible inferred Object capture/copy/Phi bounds retain exact caught evidence.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    DeclarationBindings, JavaField, JavaResolver, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::Constant,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (name, superclass) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        (
            "java.lang.NullPointerException",
            Some("java.lang.RuntimeException"),
        ),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.lang.String", Some("java.lang.Object")),
    ] {
        h.insert(Declaration {
            name: name.into(),
            superclass: superclass.map(Into::into),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}
struct Metadata {
    scope: scopes::RegionScope,
    ty: JavaType,
    hierarchy: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(self.ty.clone())
    }
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
}
struct Resolver {
    hierarchy: Hierarchy,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "java.lang.Exception",
                1 => "java.lang.String",
                3 => "java.lang.Object",
                _ => return None,
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("normal".encode_utf16().collect())
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 2 { 1 } else { 9 })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let exception = JavaType::Class("java.lang.Exception".into());
        Some(match r.index() {
            0 => JavaMethod {
                owner: "HandlerPhiHarness".into(),
                name: "raise".into(),
                is_static: true,
                parameters: vec![exception, JavaType::Int],
                return_type: JavaType::Int,
                throws: vec!["java.lang.Exception".into()],
            },
            1 => JavaMethod {
                owner: "HandlerPhiHarness".into(),
                name: "observe".into(),
                is_static: true,
                parameters: vec![exception, JavaType::Int],
                return_type: JavaType::Void,
                throws: vec![],
            },
            2 => JavaMethod {
                owner: "java.lang.Object".into(),
                name: "toString".into(),
                is_static: false,
                parameters: vec![],
                return_type: JavaType::Class("java.lang.String".into()),
                throws: vec![],
            },
            _ => return None,
        })
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        self.hierarchy.assignable(actual, expected) == Compatibility::Assignable
    }
}
struct Builder {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
}
impl Builder {
    fn new() -> Self {
        let mut insns = InsnArena::default();
        let id = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(id),
            insns,
            values: ValueArena::default(),
        }
    }
    fn block(&mut self) -> BlockId {
        self.cfg
            .add_block(self.insns.insert(ManagedInsn::Return { value: None }))
    }
    fn value(&mut self, block: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(block).unwrap().payload(), ty))
    }
    fn set(&mut self, block: BlockId, insn: ManagedInsn) {
        self.insns[*self.cfg.block(block).unwrap().payload()] = insn;
    }
    fn input(&mut self, ty: ManagedType) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        let v = self.values.insert(Value::new(id, ty));
        self.insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        };
        v
    }
    fn edge(&mut self, from: BlockId, to: BlockId, kind: EdgeKind) -> EdgeId {
        self.cfg.add_edge(from, to, kind).unwrap()
    }
    fn link(&mut self, from: BlockId, to: BlockId) -> EdgeId {
        self.edge(from, to, EdgeKind::Unconditional)
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Original,
    MissingInput,
    ConflictingCapture,
    NormalInput,
    PrethrowResult,
}
struct Fixture {
    ir: MethodIr<SsaForm>,
    metadata: Vec<Metadata>,
    resolver: Resolver,
    method: JavaMethod,
    bindings: DeclarationBindings,
    phi: ValueId,
    downstream: ValueId,
}
fn fixture(mode: Mode) -> Fixture {
    let mut b = Builder::new();
    let exception = b.input(ManagedType::Reference(ClassRef::new(0)));
    let first = b.input(ManagedType::Boolean);
    let old = b.input(ManagedType::Int);
    let entry = b.cfg.entry();
    let site_a = b.block();
    let site_b = b.block();
    let yes = b.edge(entry, site_a, EdgeKind::BranchTrue);
    let no = b.edge(entry, site_b, EdgeKind::BranchFalse);
    b.set(
        entry,
        ManagedInsn::Branch {
            condition: Some(first),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let normal = b.block();
    let normal_value = b.value(normal, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        normal,
        ManagedInsn::Const {
            result: normal_value,
            value: Constant::String(StringRef::new(0)),
        },
    );
    let normal_ret = b.block();
    b.set(
        normal_ret,
        ManagedInsn::Return {
            value: Some(normal_value),
        },
    );
    b.link(normal, normal_ret);
    let join = b.block();
    let phi = b.value(join, ManagedType::Reference(ClassRef::new(3)));
    let snapshot_join = b.block();
    let snapshot_phi = b.value(snapshot_join, ManagedType::Int);
    b.link(join, snapshot_join);
    let mut inputs = vec![];
    let mut snapshot_inputs = vec![];
    let mut metadata = vec![];
    for (index, site) in [site_a, site_b].into_iter().enumerate() {
        let result = b.value(site, ManagedType::Int);
        b.set(
            site,
            ManagedInsn::Invoke {
                kind: managed_ir::insn::InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(0),
                args: vec![exception, old],
            },
        );
        b.link(site, normal);
        let handler = b.block();
        let snapshot = b.value(handler, ManagedType::Int);
        let edge = b.edge(site, handler, EdgeKind::Exception("caught".into()));
        b.set(
            handler,
            ManagedInsn::Phi {
                result: snapshot,
                inputs: vec![(
                    edge,
                    if matches!(mode, Mode::PrethrowResult) {
                        result
                    } else {
                        old
                    },
                )],
            },
        );
        let capture = b.block();
        let caught = b.value(capture, ManagedType::Reference(ClassRef::new(3)));
        b.set(
            capture,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: EffectFlags::NONE,
            },
        );
        b.link(handler, capture);
        let copy = b.block();
        let copied = b.value(copy, ManagedType::Reference(ClassRef::new(3)));
        b.set(
            copy,
            ManagedInsn::Move {
                result: copied,
                value: caught,
            },
        );
        b.link(capture, copy);
        let merge_edge = b.link(copy, join);
        inputs.push((
            merge_edge,
            if index == 1 && matches!(mode, Mode::NormalInput) {
                exception
            } else {
                copied
            },
        ));
        snapshot_inputs.push((merge_edge, snapshot));
        metadata.push(Metadata {
            scope: scopes::RegionScope {
                protected: vec![site],
                catches: vec![scopes::Catch {
                    key: "caught".into(),
                    entry: handler,
                }],
                continuation: None,
            },
            ty: JavaType::Class(
                if index == 1 && matches!(mode, Mode::ConflictingCapture) {
                    "java.lang.NullPointerException"
                } else {
                    "java.lang.Exception"
                }
                .into(),
            ),
            hierarchy: hierarchy(),
        });
    }
    if matches!(mode, Mode::MissingInput) {
        inputs.pop();
    }
    b.set(
        join,
        ManagedInsn::Phi {
            result: phi,
            inputs,
        },
    );
    b.set(
        snapshot_join,
        ManagedInsn::Phi {
            result: snapshot_phi,
            inputs: snapshot_inputs,
        },
    );
    let copy = b.block();
    let downstream = b.value(copy, ManagedType::Reference(ClassRef::new(3)));
    b.set(
        copy,
        ManagedInsn::Move {
            result: downstream,
            value: phi,
        },
    );
    b.link(snapshot_join, copy);
    let observe = b.block();
    b.set(
        observe,
        ManagedInsn::Invoke {
            kind: managed_ir::insn::InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![downstream, snapshot_phi],
        },
    );
    b.link(copy, observe);
    let stringify = b.block();
    let string = b.value(stringify, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        stringify,
        ManagedInsn::Invoke {
            kind: managed_ir::insn::InvokeKind::Virtual,
            result: Some(string),
            method: MethodRef::new(2),
            args: vec![downstream],
        },
    );
    b.link(observe, stringify);
    let ret = b.block();
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(string),
        },
    );
    b.link(stringify, ret);
    Fixture {
        ir: MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        metadata,
        resolver: Resolver {
            hierarchy: hierarchy(),
        },
        method: JavaMethod {
            owner: "HandlerPhiHarness".into(),
            name: "generated".into(),
            is_static: true,
            return_type: JavaType::Class("java.lang.String".into()),
            parameters: vec![
                JavaType::Class("java.lang.Exception".into()),
                JavaType::Boolean,
                JavaType::Int,
            ],
            throws: vec![],
        },
        bindings: DeclarationBindings {
            parameters: vec![Some(exception), Some(first), Some(old)],
            ..Default::default()
        },
        phi,
        downstream,
    }
}
fn metadata(f: &Fixture) -> Vec<&dyn RegionMetadata> {
    f.metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect()
}
fn planned(f: &Fixture) -> Result<Plan, Error> {
    plan(
        &f.ir,
        &metadata(f),
        &f.bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &f.resolver,
        &HashMap::new(),
    )
}
fn emitted(f: &Fixture) -> SourceResult {
    emit_method(&f.ir, &f.method, &f.bindings, &f.resolver, &metadata(f))
}

#[test]
fn object_bounds_narrow_only_proven_capture_phi_copy_lineages() {
    let f = fixture(Mode::Original);
    let p = planned(&f).unwrap();
    for value in [f.phi, f.downstream] {
        assert_eq!(
            p.contextual[&value],
            JavaType::Class("java.lang.Exception".into())
        );
        assert_eq!(
            f.ir.value(value).unwrap().ty(),
            &ManagedType::Reference(ClassRef::new(3))
        );
    }
    assert!(
        matches!(emitted(&f), SourceResult::Complete { .. }),
        "{:?}",
        emitted(&f)
    );
}
#[test]
fn incomplete_conflicting_and_prethrow_result_inputs_remain_unproved() {
    for mode in [
        Mode::MissingInput,
        Mode::ConflictingCapture,
        Mode::PrethrowResult,
    ] {
        let f = fixture(mode);
        if let Ok(p) = planned(&f) {
            assert!(!p.contextual.contains_key(&f.phi));
            assert!(!p.contextual.contains_key(&f.downstream));
        }
        assert!(
            matches!(emitted(&f), SourceResult::Unsupported(_)),
            "{:?}",
            emitted(&f)
        );
    }
}
#[test]
#[ignore = "requires JDK25 ClassFile API; original disjoint-handler Phi runtime oracle"]
fn original_selected_handler_phi_matches_caught_identity_to_string_and_prethrow_snapshots() {
    let f = fixture(Mode::Original);
    let SourceResult::Complete { source, .. } = emitted(&f) else {
        panic!("{:?}", emitted(&f));
    };
    let dir = std::env::temp_dir().join(format!("original-handler-phi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class HandlerPhiHarness {{
static int mode;static String events="";static Throwable seen;static final RuntimeException stringFailure=new RuntimeException("toString"),handlerFailure=new RuntimeException("handler");
static class Probe extends Exception {{ public String toString() {{events+="T";if(mode==3)throw stringFailure;return "selected";}}}}
public static int raise(Exception e,int old)throws Exception{{events+="R";if(mode!=2)throw e;return old+1;}}
public static void observe(Exception e,int old){{seen=e;events+="H"+old;if(mode==4)throw handlerFailure;}}
static String original(Exception e,boolean first,int old){{return "placeholder";}}
{source}
public static void main(String[]args){{mode=Integer.parseInt(args[0]);boolean first=Boolean.parseBoolean(args[1]),lowered=Boolean.parseBoolean(args[2]);Exception e=new Probe();Throwable escaped=null;String result=null;try{{result=lowered?generated(e,first,7):original(e,first,7);}}catch(Throwable t){{escaped=t;}}System.out.println(result+":"+events+":"+(seen==e)+":"+(escaped==stringFailure)+":"+(escaped==handlerFailure));}}
}}"#
    );
    std::fs::write(dir.join("HandlerPhiHarness.java"), java).unwrap();
    let original = r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;public class Original{public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("HandlerPhiHarness"),exception=ClassDesc.of("java.lang.Exception");Path path=Path.of("HandlerPhiHarness.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,e)->{if(e instanceof MethodModel mm&&mm.methodName().equalsString("original"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label a=c.newLabel(),ae=c.newLabel(),b=c.newLabel(),be=c.newLabel(),ha=c.newLabel(),hb=c.newLabel(),join=c.newLabel();c.iload(1).ifeq(b);c.labelBinding(a).aload(0).iload(2).invokestatic(self,"raise",MethodTypeDesc.ofDescriptor("(Ljava/lang/Exception;I)I")).pop().labelBinding(ae).ldc("normal").areturn();c.labelBinding(b).aload(0).iload(2).invokestatic(self,"raise",MethodTypeDesc.ofDescriptor("(Ljava/lang/Exception;I)I")).pop().labelBinding(be).ldc("normal").areturn();c.labelBinding(ha).astore(3).iload(2).istore(4).goto_(join);c.labelBinding(hb).astore(5).iload(2).istore(6).aload(5).astore(3).iload(6).istore(4).goto_(join);c.labelBinding(join).aload(3).iload(4).invokestatic(self,"observe",MethodTypeDesc.ofDescriptor("(Ljava/lang/Exception;I)V")).aload(3).invokevirtual(ClassDesc.of("java.lang.Object"),"toString",MethodTypeDesc.ofDescriptor("()Ljava/lang/String;")).areturn();c.exceptionCatch(a,ae,ha,exception);c.exceptionCatch(b,be,hb,exception);});else mb.with(me);});else cb.with(e);}));}}"#;
    std::fs::write(dir.join("Original.java"), original).unwrap();
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |tool: &str, args: &[&str]| {
        let r = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        r.stdout
    };
    run(
        "javac",
        &["--release", "25", "HandlerPhiHarness.java", "Original.java"],
    );
    run("java", &["Original"]);
    let mut evidence = String::new();
    for (mode, expected) in [
        (0, "selected:RH7T:true:false:false\n"),
        (2, "normal:R:false:false:false\n"),
        (3, "null:RH7T:true:true:false\n"),
        (4, "null:RH7:true:false:true\n"),
    ] {
        let mode = mode.to_string();
        for first in ["true", "false"] {
            let before = run(
                "java",
                &[
                    "-Xverify:all",
                    "-cp",
                    ".",
                    "HandlerPhiHarness",
                    &mode,
                    first,
                    "false",
                ],
            );
            let after = run(
                "java",
                &[
                    "-Xverify:all",
                    "-cp",
                    ".",
                    "HandlerPhiHarness",
                    &mode,
                    first,
                    "true",
                ],
            );
            assert_eq!(String::from_utf8_lossy(&before), expected);
            assert_eq!(before, after);
            evidence.push_str(&format!(
                "mode={mode} first={first} original=lowered {expected}"
            ));
        }
    }
    std::fs::write(dir.join("runtime-evidence.txt"), evidence).unwrap();
    eprintln!("original selected handler Phi oracle: {}", dir.display());
}

#[test]
fn inferred_capture_bound_is_not_a_cast_or_unknown_ancestry_escape() {
    let h = hierarchy();
    let exception = JavaType::Class("java.lang.Exception".into());
    let object = JavaType::Class("java.lang.Object".into());
    let string = JavaType::Class("java.lang.String".into());
    assert_eq!(
        capture_types::domain(&exception, None, Some(&object), &h).unwrap(),
        exception
    );
    assert!(capture_types::domain(&exception, None, Some(&string), &h).is_err());
    assert!(capture_types::domain(&exception, None, Some(&JavaType::Int), &h).is_err());
    let unknown = JavaType::Class("MissingException".into());
    assert!(capture_types::domain(&unknown, None, Some(&object), &h).is_err());
    let error = JavaType::Class("java.lang.Error".into());
    assert_eq!(
        capture_types::domain(&error, Some(&exception), Some(&object), &h).unwrap(),
        JavaType::Class("java.lang.Throwable".into())
    );
    assert!(capture_types::domain(&error, Some(&exception), Some(&exception), &h).is_err());
}

#[test]
fn sibling_catches_retain_only_proven_throwable_common_inferred_bound() {
    let mut h = hierarchy();
    for name in ["java.io.IOException", "android.os.RemoteException"] {
        h.insert(Declaration {
            name: name.into(),
            superclass: Some("java.lang.Exception".into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    let ty = |name: &str| JavaType::Class(name.into());
    let io = ty("java.io.IOException");
    let remote = ty("android.os.RemoteException");
    let exception = ty("java.lang.Exception");
    let npe = ty("java.lang.NullPointerException");
    for (caught, previous) in [
        (&io, &remote),
        (&remote, &io),
        (&npe, &remote),
        (&remote, &npe),
    ] {
        assert_eq!(
            super::capture_types::domain(caught, Some(previous), Some(&exception), &h).unwrap(),
            exception
        );
        // Object must never turn caught evidence into a non-Throwable domain.
        assert_eq!(
            super::capture_types::domain(caught, Some(previous), Some(&ty("java.lang.Object")), &h)
                .unwrap(),
            ty("java.lang.Throwable")
        );
        assert!(super::capture_types::domain(caught, Some(previous), Some(&io), &h).is_err());
        assert!(
            super::capture_types::domain(caught, Some(previous), Some(&ty("missing.Common")), &h)
                .is_err()
        );
    }
    // Error and Exception siblings cannot be narrowed to Exception.
    assert!(
        super::capture_types::domain(&ty("java.lang.Error"), Some(&io), Some(&exception), &h)
            .is_err()
    );
    assert!(
        super::capture_types::domain(&io, Some(&ty("java.lang.Error")), Some(&exception), &h)
            .is_err()
    );
}

#[test]
fn typed_parameter_join_preserves_original_selected_identity_and_effects() {
    super::trusted_normal_tests::assert_source(emitted(&fixture(Mode::NormalInput)));
}
