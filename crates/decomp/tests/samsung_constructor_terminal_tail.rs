//! Original Samsung constructors, genuine metadata, and exact runtime fixtures.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorInput, ConstructorResolver, Initializer,
    TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_class_kinds::initializers::{self, write_inventory::WriteInventory};
use decomp::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use dex_format::{DexFile, class::ClassDef};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
use std::path::PathBuf;

fn descriptor(dex: &DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn source_name(descriptor: &str) -> String {
    descriptor
        .trim_start_matches('L')
        .trim_end_matches(';')
        .replace('/', ".")
}
fn method_key(dex: &DexFile, index: u32) -> decomp::java_aliases::MethodKey {
    let raw = dex.method_id(index).unwrap();
    let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
    decomp::java_aliases::MethodKey {
        owner: descriptor(dex, u32::from(raw.class_idx)).into(),
        name: dex.string(raw.name_idx).unwrap().into(),
        parameters: proto
            .parameters
            .as_ref()
            .into_iter()
            .flat_map(|p| &p.types)
            .map(|i| descriptor(dex, u32::from(*i)).into())
            .collect(),
        return_type: descriptor(dex, proto.return_type_idx).into(),
    }
}
struct Resolver<'a> {
    inputs: &'a [DexFile],
    dex: &'a DexFile,
    class: &'a ClassDef,
    cp: &'a Classpath,
    super_proof: bool,
}
impl Resolver<'_> {
    fn declaration(&self, key: &decomp::java_aliases::MethodKey) -> Option<MethodMetadata> {
        for dex in self.inputs {
            if let Some(class) = dex
                .classes
                .iter()
                .find(|c| descriptor(dex, c.class_idx) == key.owner)
            {
                let data = class.class_data.as_ref()?;
                let m = data
                    .direct_methods
                    .iter()
                    .chain(&data.virtual_methods)
                    .find(|m| method_key(dex, m.method_idx) == *key)?;
                let exceptions = decomp::java_members::MemberResolver { dex }
                    .declared_throws_for_class(class, m.method_idx)?
                    .iter()
                    .map(|name| format!("L{};", name.replace('.', "/")))
                    .collect();
                return Some(MethodMetadata {
                    access_flags: m.access_flags,
                    exceptions: Some(exceptions),
                });
            }
        }
        self.cp.method(key)
    }
}
impl JavaResolver for Resolver<'_> {
    fn instruction_type(&self, r: managed_ir::refs::TypeRef) -> Option<JavaType> {
        decomp::dex_java::descriptor_type(descriptor(self.dex, r.index()))
    }

    fn super_invocation(
        &self,
        caller: &str,
        reference: MethodRef,
    ) -> Option<backend_java::SuperInvocation> {
        if !self.constructor_super_target(caller, reference) {
            return None;
        }
        Some(backend_java::SuperInvocation {
            method: self.method(reference)?,
            access_flags: self.method_access(reference)?,
        })
    }
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        let key = method_key(self.dex, reference.index());
        let metadata = self.declaration(&key)?;
        Some(JavaMethod {
            owner: source_name(&key.owner),
            name: key.name,
            is_static: metadata.access_flags & 8 != 0,
            return_type: decomp::dex_java::descriptor_type(&key.return_type)?,
            parameters: key
                .parameters
                .iter()
                .map(|p| decomp::dex_java::descriptor_type(p))
                .collect::<Option<_>>()?,
            throws: metadata
                .exceptions?
                .iter()
                .map(|d| source_name(d))
                .collect(),
        })
    }
    fn method_access(&self, reference: MethodRef) -> Option<u32> {
        Some(
            self.declaration(&method_key(self.dex, reference.index()))?
                .access_flags,
        )
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        let raw = self.dex.field_id(reference.index()).ok()?;
        let data = self.class.class_data.as_ref()?;
        let declared = data
            .instance_fields
            .iter()
            .chain(&data.static_fields)
            .find(|f| f.field_idx == reference.index())?;
        Some(JavaField {
            owner: self.class(ClassRef::new(u32::from(raw.class_idx)))?,
            name: self.dex.string(raw.name_idx).ok()?.into(),
            ty: decomp::dex_java::descriptor_type(descriptor(self.dex, u32::from(raw.type_idx)))?,
            is_static: declared.access_flags & 8 != 0,
        })
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        match decomp::dex_java::descriptor_type(descriptor(self.dex, reference.index()))? {
            JavaType::Class(name) => Some(name),
            _ => None,
        }
    }
    fn string(&self, reference: StringRef) -> Option<Vec<u16>> {
        Some(
            self.dex
                .string(reference.index())
                .ok()?
                .encode_utf16()
                .collect(),
        )
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        if actual == expected
            || matches!((actual,expected),(JavaType::Array(_),JavaType::Class(n)) if n=="java.lang.Object")
        {
            return true;
        }
        let (JavaType::Class(actual), JavaType::Class(expected)) = (actual, expected) else {
            return false;
        };
        let mut next = Some(actual.clone());
        let mut seen = std::collections::BTreeSet::new();
        while let Some(name) = next {
            if name == *expected {
                return true;
            }
            if !seen.insert(name.clone()) {
                return false;
            }
            next = self.constructor_class(&name).and_then(|c| c.superclass);
        }
        false
    }
}
impl ConstructorResolver for Resolver<'_> {
    fn initializer(&self, reference: MethodRef) -> Option<Initializer> {
        let method = self.method(reference)?;
        if method.name != "<init>" {
            return None;
        }
        let delegates_to = self
            .class
            .class_data
            .as_ref()?
            .direct_methods
            .iter()
            .find(|m| m.method_idx == reference.index())
            .and_then(|m| m.code.as_ref())
            .and_then(|code| dex_decompiler::decode_method(code).ok())
            .and_then(|instructions| {
                instructions.into_iter().find_map(|i| {
                    let dex_decompiler::Opcode::Supported(info) = i.opcode else {
                        return None;
                    };
                    if ![0x70, 0x76].contains(&info.opcode) {
                        return None;
                    }
                    let target = i.pool_index?;
                    let raw = self.dex.method_id(target).ok()?;
                    (u32::from(raw.class_idx) == self.class.class_idx
                        && self.dex.string(raw.name_idx).ok()? == "<init>")
                        .then_some(MethodRef::new(target))
                })
            });
        Some(Initializer {
            method,
            delegates_to,
        })
    }
    fn constructor_super_target(&self, caller: &str, reference: MethodRef) -> bool {
        if !self.super_proof || caller != source_name(descriptor(self.dex, self.class.class_idx)) {
            return false;
        }
        let key = method_key(self.dex, reference.index());
        if key.owner != descriptor(self.dex, self.class.superclass_idx) {
            return false;
        }
        let Some(parent) = self.cp.class(&key.owner) else {
            return false;
        };
        let mut witnessed = false;
        for method in &self.class.class_data.as_ref().unwrap().direct_methods {
            let Some(code) = &method.code else {
                continue;
            };
            for insn in dex_decompiler::decode_method(code).unwrap() {
                let dex_decompiler::Opcode::Supported(info) = insn.opcode else {
                    continue;
                };
                if insn.pool_index == Some(reference.index())
                    && [0x6f, 0x70, 0x75, 0x76].contains(&info.opcode)
                {
                    if ![0x6f, 0x75].contains(&info.opcode) {
                        return false;
                    }
                    witnessed = true;
                }
            }
        }
        // Exact direct-parent original declaration, no source alias or generic/
        // varargs competitor. Descriptor casts pin Java overload resolution.
        witnessed
            && parent.methods.iter().any(|m| {
                m.key == key
                    && m.signature.is_none()
                    && m.metadata.access_flags & (2 | 8 | 0x80 | 0x400) == 0
            })
            && parent
                .methods
                .iter()
                .filter(|m| m.key.name == key.name)
                .all(|m| m.signature.is_none() && m.metadata.access_flags & 0x80 == 0)
    }
}
impl TrustedConstructorResolver for Resolver<'_> {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        for dex in self.inputs {
            if let Some(class) = dex
                .classes
                .iter()
                .find(|c| source_name(descriptor(dex, c.class_idx)) == name)
            {
                return Some(ConstructorClass {
                    name: name.into(),
                    superclass: (class.superclass_idx != u32::MAX)
                        .then(|| source_name(descriptor(dex, class.superclass_idx))),
                    access_flags: class.access_flags,
                });
            }
        }
        let class = self.cp.class(&format!("L{};", name.replace('.', "/")))?;
        Some(ConstructorClass {
            name: name.into(),
            superclass: class.declaration.superclass.clone(),
            access_flags: class.access_flags,
        })
    }
    fn constructor_access(&self, reference: MethodRef) -> Option<u32> {
        let key = method_key(self.dex, reference.index());
        (key.name == "<init>")
            .then(|| self.declaration(&key).map(|m| m.access_flags))
            .flatten()
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        if source_name(descriptor(self.dex, self.class.class_idx)) != owner {
            return None;
        }
        Some(
            self.class
                .class_data
                .as_ref()?
                .direct_methods
                .iter()
                .filter(|m| method_key(self.dex, m.method_idx).name == "<init>")
                .map(|m| MethodRef::new(m.method_idx))
                .collect(),
        )
    }
}

fn java_home() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
}

#[test]
#[ignore = "requires retained Samsung APK and explicit JDK/Android metadata"]
fn samsung_terminal_shared_constructor_original_identity() {
    let inputs = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        java_home().join("jmods/java.base.jmod"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    let (di, dex, class) = inputs
        .iter()
        .enumerate()
        .find_map(|(i, d)| {
            d.classes
                .iter()
                .find(|c| descriptor(d, c.class_idx) == "Lag/b;")
                .map(|c| (i, d, c))
        })
        .unwrap();
    let r = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        super_proof: true,
    };
    let raw = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 14)
        .unwrap();
    let code = raw.code.as_ref().unwrap();
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        code,
        &dex_decompiler::DexTypes::new(dex, Some(raw.method_idx)),
    )
    .unwrap();
    let signature = r.method(MethodRef::new(14)).unwrap();
    let bindings = decomp::dex_java::parameter_bindings(
        ssa.ir(),
        &signature,
        code.registers_size,
        code.ins_size,
    )
    .unwrap();
    let original = ConstructorInput {
        reference: MethodRef::new(14),
        ir: ssa.ir(),
        bindings: &bindings,
    };
    let proof = initializers::prove_instance_final_explicit_stores(
        &inputs,
        di,
        class,
        &std::collections::BTreeSet::from([14]),
        &inventory,
    )
    .unwrap();
    let fields = proof.plan.iter().map(|f| f.reference).collect::<Vec<_>>();
    let evidence = backend_java::constructors::constructor_initialization_evidence(
        &original,
        "java.lang.Object",
        &fields,
        &r,
    )
    .unwrap();
    assert!(evidence.fields.iter().all(|f| f.normal_return_counts
        == std::collections::BTreeSet::from([1])
        && !f.read_before_assignment));
    let plan = backend_java::constructors::plan_constructor(
        ssa.ir(),
        original.reference,
        "java.lang.Object",
        &bindings,
        &r,
    )
    .unwrap();
    assert!(plan.source.contains("while (true)"));
    assert_eq!(plan.source.matches("this.b =").count(), 4);
    let dir = std::env::temp_dir().join(format!(
        "decomphify-wave26-terminal-tail-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(dir.join("ag")).unwrap();
    // Boundary fixture only: ag.a is never inspected/called by the actual ctor.
    // This is not a whole original class/APK compilation claim.
    std::fs::write(dir.join("ag/a.java"), "package ag; public class a {}").unwrap();
    std::fs::write(
        dir.join("ag/b.java"),
        format!(
            "package ag; public final class b {{ public final a a; public final int[] b; {} }}",
            plan.source
        ),
    )
    .unwrap();
    let harness = r#"import ag.b; public class Check {
 static int[] expected(int[] v){if(v.length==0)throw new IllegalArgumentException();if(v.length<=1||v[0]!=0)return v;int i=1;while(i<v.length&&v[i]==0)i++;if(i==v.length)return new int[]{0};int[] t=new int[v.length-i];System.arraycopy(v,i,t,0,t.length);return t;}
 public static void main(String[]args){
 for(int[]v:new int[][]{{0},{7},{0,0},{0,0,0},{0,1},{0,0,7,0},{4,0,9},{0,-2,3}}){int[]want=expected(v);b got=new b(null,v);if(!java.util.Arrays.equals(want,got.b))throw new AssertionError();if((want==v)!=(got.b==v))throw new AssertionError("alias");if(got.a!=null)throw new AssertionError();}
 try{new b(null,new int[0]);throw new AssertionError();}catch(IllegalArgumentException ok){}
 try{new b(null,null);throw new AssertionError();}catch(NullPointerException ok){}
 ag.a owner=new ag.a();if(new b(owner,new int[]{3}).a!=owner)throw new AssertionError("owner identity");
 }}"#;
    std::fs::write(dir.join("Check.java"), harness).unwrap();
    for (binary, args) in [
        (
            "javac",
            vec!["--release", "17", "ag/a.java", "ag/b.java", "Check.java"],
        ),
        ("java", vec!["-cp", ".", "Check"]),
    ] {
        let out = std::process::Command::new(java_home().join("bin").join(binary))
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
