//! Selected original Auth method: local allocation scope and body claims are separate.
#![allow(clippy::unwrap_used)]
use crate::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use crate::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
use std::path::PathBuf;
const APK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
);
const OWNER: &str = "Lcom/samsung/android/samsungpass/sdk/SamsungPassAuthService;";
fn artifact_root() -> PathBuf {
    std::env::var_os("DECOMPHIFY_AUTH_SCOPE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/wave8-auth-input-copy".into())
}
fn jdk_classpath() -> PathBuf {
    std::env::var_os("DECOMPHIFY_JDK25_CLASSPATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/decomphify-source25-java-base-649174/java.base.jar".into())
}

fn descriptor(dex: &dex_format::DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn source_name(descriptor: &str) -> String {
    descriptor
        .trim_start_matches('L')
        .trim_end_matches(';')
        .replace('/', ".")
}
fn method_key(dex: &dex_format::DexFile, index: u32) -> crate::java_aliases::MethodKey {
    let m = dex.method_id(index).unwrap();
    let p = dex.proto_id(u32::from(m.proto_idx)).unwrap();
    crate::java_aliases::MethodKey {
        owner: descriptor(dex, u32::from(m.class_idx)).into(),
        name: dex.string(m.name_idx).unwrap().into(),
        parameters: p
            .parameters
            .as_ref()
            .into_iter()
            .flat_map(|p| &p.types)
            .map(|i| descriptor(dex, u32::from(*i)).into())
            .collect(),
        return_type: descriptor(dex, p.return_type_idx).into(),
    }
}
struct ScopeResolver<'a> {
    inputs: &'a [dex_format::DexFile],
    dex: &'a dex_format::DexFile,
    cp: &'a Classpath,
}
impl ScopeResolver<'_> {
    fn declaration(&self, key: &crate::java_aliases::MethodKey) -> Option<MethodMetadata> {
        for dex in self.inputs {
            if let Some(class) = dex
                .classes
                .iter()
                .find(|c| descriptor(dex, c.class_idx) == key.owner)
            {
                let data = class.class_data.as_ref()?;
                let method = data
                    .direct_methods
                    .iter()
                    .chain(&data.virtual_methods)
                    .find(|m| method_key(dex, m.method_idx) == *key)?;
                let throws = crate::java_members::MemberResolver { dex }
                    .declared_throws_for_class(class, method.method_idx)?
                    .into_iter()
                    .map(|name| format!("L{};", name.replace('.', "/")))
                    .collect();
                return Some(MethodMetadata {
                    access_flags: method.access_flags,
                    exceptions: Some(throws),
                });
            }
        }
        self.cp.method(key)
    }
}
impl JavaResolver for ScopeResolver<'_> {
    fn class(&self, r: ClassRef) -> Option<String> {
        match crate::dex_java::descriptor_type(descriptor(self.dex, r.index()))? {
            JavaType::Class(name) => Some(name),
            _ => None,
        }
    }
    fn instruction_type(&self, r: managed_ir::refs::TypeRef) -> Option<JavaType> {
        crate::dex_java::descriptor_type(descriptor(self.dex, r.index()))
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let key = method_key(self.dex, r.index());
        let metadata = self.declaration(&key)?;
        Some(JavaMethod {
            owner: source_name(&key.owner),
            name: key.name,
            is_static: metadata.access_flags & 8 != 0,
            return_type: crate::dex_java::descriptor_type(&key.return_type)?,
            parameters: key
                .parameters
                .iter()
                .map(|p| crate::dex_java::descriptor_type(p))
                .collect::<Option<_>>()?,
            throws: metadata
                .exceptions?
                .iter()
                .map(|d| source_name(d))
                .collect(),
        })
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(
            self.declaration(&method_key(self.dex, r.index()))?
                .access_flags,
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Some(self.dex.string(r.index()).ok()?.encode_utf16().collect())
    }
}
#[test]
#[ignore = "requires retained Samsung APK and genuine JDK25/Android35 metadata; local scope proof"]
fn original_auth_20398_nine_exact_allocation_scopes() {
    use backend_java::exceptions::RegionMetadata;
    use dex_decompiler::infer::TypeContext;
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[2];
    let k = method_key(dex, 20398);
    assert_eq!(k.owner, OWNER);
    assert_eq!(k.name, "doSvcAuthOperation");
    assert_eq!(
        k.parameters,
        [
            "I",
            "Lcom/samsung/android/pass/IPassListener;",
            "Ljava/lang/String;",
            "[B"
        ]
    );
    assert_eq!(k.return_type, "V");
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == OWNER)
        .unwrap();
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 20398)
        .unwrap();
    assert_eq!(method.access_flags, 2);
    let code = method.code.as_ref().unwrap();
    assert_eq!(
        (code.registers_size, code.ins_size, code.insns_size),
        (15, 5, 284)
    );
    assert_eq!(
        code.tries
            .iter()
            .map(|t| (
                t.start_addr,
                t.start_addr + u32::from(t.insn_count),
                t.handler_off
            ))
            .collect::<Vec<_>>(),
        [(2, 46, 1), (69, 97, 1), (100, 125, 8), (128, 221, 1)]
    );
    let cp = Classpath::from_paths([
        jdk_classpath(),
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
    ])
    .unwrap();
    let resolver = ScopeResolver {
        inputs: &inputs,
        dex,
        cp: &cp,
    };
    let hierarchy =
        crate::java_hierarchy::build_hierarchy(&inputs.iter().collect::<Vec<_>>(), cp.hierarchy())
            .unwrap();
    let context = dex_decompiler::DexTypes::new(dex, Some(20398));
    let raw = dex.method_id(20398).unwrap();
    let mut seeds = std::collections::HashMap::new();
    let mut register = code.registers_size - code.ins_size;
    seeds.insert(
        (register, false),
        context.type_at(u32::from(raw.class_idx)).unwrap(),
    );
    register += 1;
    let mut parameters = vec![];
    assert!(context.method_params(20398, &mut parameters));
    for ty in parameters {
        let wide = matches!(
            ty,
            managed_ir::types::ManagedType::Long | managed_ir::types::ManagedType::Double
        );
        seeds.insert((register, wide), ty);
        register += if wide { 2 } else { 1 };
    }
    assert_eq!(register, code.registers_size);
    let ssa = dex_decompiler::DexPasses::run_ssa_with(code, &context.with_initial_registers(seeds))
        .unwrap();
    let metadata =
        crate::java_exception_regions::from_dex_dispatch(&ssa, code, &resolver, &hierarchy)
            .unwrap();
    let scopes = metadata
        .scopes
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let proved = backend_java::exceptions::original_allocation_chains(ssa.ir(), &scopes, &resolver);
    let pairs = proved
        .iter()
        .map(|c| {
            (
                ssa.offset(c[0]).unwrap(),
                ssa.offset(*c.last().unwrap()).unwrap(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        pairs,
        std::collections::BTreeSet::from([
            (4, 6),
            (69, 71),
            (89, 93),
            (109, 119),
            (130, 132),
            (162, 169),
            (176, 183),
            (194, 201),
            (212, 215)
        ])
    );
    assert_eq!(proved.len(), 9);
    let root = artifact_root();
    std::fs::create_dir_all(&root).unwrap();
    let evidence = proved
        .iter()
        .map(|c| c.iter().map(|i| ssa.offset(*i)).collect::<Vec<_>>())
        .collect::<std::collections::BTreeSet<_>>();
    std::fs::write(root.join("genuine-nine-chain-scope-proof.txt"),format!("identity={OWNER}->doSvcAuthOperation(ILcom/samsung/android/pass/IPassListener;Ljava/lang/String;[B)V\ndex=classes3.dex\nmethod=20398\nflags=0x2\noriginal_scopes={:?}\noriginal_chains={evidence:?}\nThis is local range/dispatch/handler proof, not method-body success.\n",code.tries)).unwrap();
    eprintln!("original classes3.dex20398 genuine chains={evidence:?}");
}
#[test]
#[ignore = "requires retained Samsung APK and genuine JDK25/Android35 metadata; one selected class"]
fn original_auth_20398_facade_body_evidence() {
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[2];
    let method = dex.method_id(20398).unwrap();
    assert_eq!(
        dex.string(
            dex.type_id(u32::from(method.class_idx))
                .unwrap()
                .descriptor_idx
        )
        .unwrap(),
        OWNER
    );
    assert_eq!(dex.string(method.name_idx).unwrap(), "doSvcAuthOperation");
    let root = artifact_root();
    std::fs::create_dir_all(&root).unwrap();
    let options = JavaOptions {
        source_release: 25,
        class_filter: Some(OWNER.into()),
        workers: 1,
        classpath: vec![
            jdk_classpath(),
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
        ],
        ..JavaOptions::default()
    };
    let mut units = vec![];
    let report = emit_java(&inputs, &options, &mut |u: &GeneratedSource| {
        units.push(u.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(units.len(), 1);
    let unit = &units[0];
    assert_eq!(unit.provenance.descriptor, OWNER);
    let path = root.join("original-units").join(&unit.relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &unit.text).unwrap();
    std::fs::write(root.join("facade-report.txt"), format!("{report:?}\n")).unwrap();
    retain_body_evidence(&unit.text, &path);
}

fn retain_body_evidence(text: &str, path: &std::path::Path) {
    let root = artifact_root();
    let marker = "    // original method pool index 20398;";
    let unavailable = "    // unavailable original method index 20398:";
    let record = text
        .lines()
        .find(|line| line.starts_with(marker) || line.starts_with(unavailable))
        .unwrap();
    assert!(record.contains("doSvcAuthOperation"));
    let body = if record.starts_with(unavailable) {
        record
    } else {
        let start = text.find(record).unwrap();
        let tail = &text[start..];
        let end = tail[record.len()..]
            .find("    // original method pool index ")
            .map(|offset| offset + record.len())
            .unwrap_or(tail.len());
        &tail[..end]
    };
    std::fs::write(root.join("original-method-20398.java.txt"), body).unwrap();
    std::fs::write(root.join("facade-body-evidence.txt"), format!("identity={record}\nsource={}\nThis terminal body evidence is independent of local scope proof.\n", path.display())).unwrap();
    eprintln!(
        "original classes3.dex #20398 {record}\nsource={}",
        path.display()
    );
    if record.contains("complete_body=true") {
        assert!(body.contains("new "));
        assert!(body.contains("catch (") || body.contains("catch("));
    }
}

#[test]
#[ignore = "reads retained selected original facade artifact; does not repeat facade work"]
fn original_auth_20398_retained_facade_body_evidence() {
    let path = artifact_root()
        .join("original-units/com/samsung/android/samsungpass/sdk/SamsungPassAuthService.java");
    retain_body_evidence(&std::fs::read_to_string(&path).unwrap(), &path);
}
