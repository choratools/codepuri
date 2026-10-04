//! Executable DEX synchronization versus declaration-only metadata.
#![allow(clippy::unwrap_used)]
use decomp::java_method_flags::*;
fn input(flags: u32, code: bool) -> MethodFlagInput {
    MethodFlagInput {
        access_flags: flags,
        owner: MethodOwner::Class { is_abstract: false },
        kind: MethodKind::Ordinary,
        has_code: code,
        last_parameter_is_array: false,
    }
}
#[test]
fn declared_metadata_does_not_add_lock_or_consume_body_monitors() {
    let p = plan_body_modifiers(input(0x20001, true)).unwrap();
    assert_eq!(p.execution, SynchronizationExecution::ExplicitBodyMonitors);
    assert!(!p.modifiers.contains(&"synchronized"));
    assert!(p.declared_synchronized_metadata);
    let p = plan_body_modifiers(input(0x129, false)).unwrap();
    assert_eq!(p.execution, SynchronizationExecution::NativeImplicitMonitor);
    assert!(p.modifiers.contains(&"synchronized"));
    assert!(plan_body_modifiers(input(0x21, true)).is_err());
    assert!(plan_body_modifiers(input(0x20101, false)).is_err());
}
#[test]
fn preserved_explicit_monitor_releases_normal_throw_and_static_null_receiver() {
    let root =
        std::env::temp_dir().join(format!("decomphify-sync-modifiers-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = r#"public class SyncModifiers {
 static final RuntimeException original=new RuntimeException();
 int value;
 int instance(boolean fail) { synchronized(this) { if(!Thread.holdsLock(this))throw new AssertionError();if(fail)throw original;return ++value; } }
 static int statik(boolean fail) { synchronized(SyncModifiers.class) { if(!Thread.holdsLock(SyncModifiers.class))throw new AssertionError();if(fail)throw original;return 7; } }
 static int metadataOnly() { if(Thread.holdsLock(SyncModifiers.class))throw new AssertionError("introduced lock");return 8; }
 public static void main(String[] args)throws Exception {
 SyncModifiers x=new SyncModifiers();if(x.instance(false)!=1||Thread.holdsLock(x))throw new AssertionError();
 try{x.instance(true);throw new AssertionError();}catch(RuntimeException e){if(e!=original||Thread.holdsLock(x))throw new AssertionError();}
 Thread t=new Thread(()->{synchronized(x){x.value++;}});t.start();t.join(2000);if(t.isAlive()||x.value!=2)throw new AssertionError();
 SyncModifiers absent=null;if(absent.statik(false)!=7||absent.metadataOnly()!=8)throw new AssertionError();
 try{absent.statik(true);throw new AssertionError();}catch(RuntimeException e){if(e!=original||Thread.holdsLock(SyncModifiers.class))throw new AssertionError();}
 Thread u=new Thread(()->{synchronized(SyncModifiers.class){}});u.start();u.join(2000);if(u.isAlive())throw new AssertionError();
 }
}"#;
    std::fs::write(root.join("SyncModifiers.java"), source).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    assert!(
        std::process::Command::new(format!("{jdk}/javac"))
            .arg(root.join("SyncModifiers.java"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new(format!("{jdk}/java"))
            .args(["-cp", root.to_str().unwrap(), "SyncModifiers"])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
#[ignore = "Requires original Samsung Pass APK"]
fn actual_samsung_declared_monitor_inventory() {
    let dexes = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let mut methods = Vec::new();
    let mut native = 0;
    for (di, d) in dexes.iter().enumerate() {
        for c in &d.classes {
            if let Some(data) = &c.class_data {
                for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                    if m.access_flags & 0x20 != 0 {
                        native += 1;
                        assert_ne!(m.access_flags & 0x100, 0);
                    }
                    if m.access_flags & 0x20000 == 0 {
                        continue;
                    }
                    let Some(code) = &m.code else {
                        panic!("managed synchronization without code")
                    };
                    let decoded = dex_decompiler::decode_method_recovering(code, 2);
                    let monitors: Vec<_> = decoded
                        .insns
                        .iter()
                        .filter_map(|i| match i.opcode {
                            dex_decompiler::decode::Opcode::Supported(op)
                                if op.opcode == 0x1d || op.opcode == 0x1e =>
                            {
                                Some((i.offset, op.opcode, i.registers.clone()))
                            }
                            _ => None,
                        })
                        .collect();
                    let mid = d.method_id(m.method_idx).unwrap();
                    let owner = d
                        .string(d.type_id(u32::from(mid.class_idx)).unwrap().descriptor_idx)
                        .unwrap();
                    let name = d.string(mid.name_idx).unwrap();
                    methods.push((
                        code.insns_size,
                        di,
                        m.method_idx,
                        m.access_flags,
                        code.span.offset,
                        format!("{owner}->{name}"),
                        monitors,
                        code.tries.len(),
                    ));
                }
            }
        }
    }
    methods.sort_by_key(|m| m.0);
    println!(
        "declared_managed={} native_implicit={native}",
        methods.len()
    );
    for m in methods.iter().take(12) {
        println!("{m:?}");
    }
    assert!(methods.len() >= 3);
}

struct Identity;
impl backend_java::JavaResolver for Identity {
    fn assignable(&self, a: &backend_java::JavaType, e: &backend_java::JavaType) -> bool {
        a == e
    }
    fn method(&self, _: managed_ir::refs::MethodRef) -> Option<backend_java::JavaMethod> {
        None
    }
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, _: managed_ir::refs::ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
}
#[test]
#[ignore = "Requires original Samsung Pass APK"]
fn three_real_empty_monitor_bodies_are_fully_proved() {
    use backend_java::JavaResolver;
    let dexes = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    for (di, index) in [(0, 40173), (0, 40175), (2, 36525)] {
        let d = &dexes[di];
        let method = d
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        let metadata = decomp::java_members::MemberResolver { dex: d }
            .method(managed_ir::refs::MethodRef::new(index))
            .unwrap();
        let code = method.code.as_ref().unwrap();
        let receiver = backend_java::JavaType::Class(metadata.owner.clone());
        let proof = empty_instance_monitor_scaffold(
            input(method.access_flags, true),
            code,
            &metadata.return_type,
            &receiver,
            &Identity,
        )
        .unwrap();
        assert_eq!(proof, EmptySynchronizedBody::Void);
        println!(
            "proved dex{di} method{index} {}.{} flags={:#x} code={}",
            metadata.owner, metadata.name, method.access_flags, code.span.offset
        );
        let mut mutated = code.clone();
        mutated.insns[0] = 0x00;
        assert!(
            empty_instance_monitor_scaffold(
                input(method.access_flags, true),
                &mutated,
                &metadata.return_type,
                &receiver,
                &Identity
            )
            .is_err()
        );
    }
}
