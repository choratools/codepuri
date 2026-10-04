#![allow(clippy::unwrap_used)]
use super::*;
use crate::java_external_metadata::{ExternalMetadata, MetadataProvider};
use std::{fs, path::PathBuf, process::Command, sync::OnceLock};

struct Fixture {
    cp: Classpath,
    path: PathBuf,
    home: PathBuf,
}
fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let home = std::env::var_os("DECOMPHIFY_JDK17_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"));
        let path = PathBuf::from("/tmp/opencode/wave12-external-default/jvm-fixture");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("DefaultFixture.java"), r#"
import java.io.IOException;
import java.util.zip.*;
public class DefaultFixture {
  static String events = "";
  static IOException fault = new IOException("sentinel");
  public interface Root {
    default int apply(byte[] b) throws IOException {
      events += "R";
      if (b == null) throw fault;
      return b.length;
    }
  }
  public interface Left extends Root {}
  public interface Right extends Root {}
  public interface More extends Root {
    default int apply(byte[] b) throws IOException { events += "M"; return b.length + 1; }
  }
  public interface AbstractChild extends Root { int apply(byte[] b) throws IOException; }
  public interface UnrelatedAbstract { int apply(byte[] b) throws IOException; }
  public interface PrivateOnly { private int apply(byte[] b) { return 1; } }
  public interface StaticOnly { static int apply(byte[] b) { return 2; } }
  public interface Generic<T> { default int apply(byte[] b) { return 3; } }
  public static class Diamond implements Left, Right {}
  public static class Most implements Root, More {}
  public static abstract class AbstractClass implements AbstractChild {}
  public static class Own implements Root { public int apply(byte[] b) { return -1; } }
  public static class OwnChild extends Own implements More {}
  public static class PrivateClass implements PrivateOnly {}
  public static class StaticClass implements StaticOnly {}
  public static class GenericClass implements Generic<String> {}
  static long crc(byte[] b) { CRC32 c = new CRC32(); c.update(b); return c.getValue(); }
  static int inherited(Diamond d, byte[] b) throws IOException { return d.apply(b); }
  public static void main(String[] args) throws Exception {
    if (CRC32.class.getMethod("update", byte[].class).getDeclaringClass() != Checksum.class)
      throw new AssertionError("actual declaration");
    try { CRC32.class.getDeclaredMethod("update", byte[].class); throw new AssertionError(); }
    catch (NoSuchMethodException expected) {}
    if (!Checksum.class.getDeclaredMethod("update", byte[].class).isDefault()) throw new AssertionError();
    for (int n = 0; n < 25; n++) {
      byte[] b = new byte[n];
      for (int i = 0; i < n; i++) b[i] = (byte)(i*43+n);
      CRC32 ref = new CRC32(); for (byte x: b) ref.update(x & 255);
      if (crc(b) != ref.getValue()) throw new AssertionError("CRC result");
    }
    if (crc("123456789".getBytes(java.nio.charset.StandardCharsets.US_ASCII)) != 0xcbf43926L)
      throw new AssertionError("known CRC");
    try { crc(null); throw new AssertionError(); }
    catch (NullPointerException e) {
      if (!java.util.Arrays.stream(e.getStackTrace()).anyMatch(s -> s.getClassName().equals("java.util.zip.Checksum") && s.getMethodName().equals("update")))
        throw new AssertionError("lost actual default frame");
    }
    Diamond d = new Diamond();
    if (inherited(d, new byte[3]) != 3 || !events.equals("R")) throw new AssertionError();
    try { inherited(d, null); throw new AssertionError(); }
    catch (IOException e) {
      if (e != fault || !events.equals("RR") || !e.getMessage().equals("sentinel")) throw new AssertionError("identity/effects");
    }
    if (new Most().apply(new byte[3]) != 4 || !events.equals("RRM")) throw new AssertionError("specific default");
    if (new OwnChild().apply(null) != -1 || !events.equals("RRM")) throw new AssertionError("class precedence");
    System.out.println("CRC32->Checksum.update([B)V;26 CRC cases;NPE default frame;checked sentinel same;effects=RRM;class dominance");
  }
}
"#).unwrap();
        let output = Command::new(home.join("bin/javac"))
            .args(["--release", "17", "DefaultFixture.java"])
            .current_dir(&path).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let mut cp = Classpath::from_paths([home.join("jmods/java.base.jmod")]).unwrap();
        for entry in fs::read_dir(&path).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "class") {
                cp.add_path(path).unwrap();
            }
        }
        Fixture { cp, path, home }
    })
}
fn key(owner: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: "apply".into(),
        parameters: vec!["[B".into()],
        return_type: "I".into(),
    }
}
fn crc_key() -> MethodKey {
    MethodKey {
        owner: "Ljava/util/zip/CRC32;".into(),
        name: "update".into(),
        parameters: vec!["[B".into()],
        return_type: "V".into(),
    }
}
#[test]
fn genuine_jdk_default_declaration_and_unchanged_symbolic_runtime_contract() {
    let f = fixture();
    assert!(
        f.cp.method(&crc_key()).is_none(),
        "method remains exact declared-only"
    );
    let selected = resolve_inherited_public_instance(&f.cp, &crc_key()).unwrap();
    assert_eq!(selected.reference(), &crc_key());
    assert_eq!(selected.declaration().key.owner, "Ljava/util/zip/Checksum;");
    assert_eq!(selected.declaration().metadata.access_flags, 1);
    assert_eq!(selected.declaration().metadata.exceptions, Some(vec![]));
    let cloned = selected.clone();
    assert!(std::ptr::eq(selected.declaration(), cloned.declaration()));
    assert!(std::ptr::eq(
        selected.declaration().code.as_ref().unwrap().bytes.as_ptr(),
        cloned.declaration().code.as_ref().unwrap().bytes.as_ptr(),
    ));
    assert_eq!(
        selected.declaration().code.as_ref().unwrap().bytes,
        [
            0x2a, 0x2b, 0x03, 0x2b, 0xbe, 0xb9, 0x00, 0x01, 0x04, 0x00, 0xb1
        ]
    );
    let sdk =
        Classpath::from_paths(["/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"])
            .unwrap();
    let chain = MetadataChain::new(vec![
        MetadataProvider::Jvm(&f.cp),
        MetadataProvider::Jvm(&sdk),
    ]);
    assert_eq!(
        resolve_inherited_public_instance(&chain, &crc_key()).unwrap(),
        selected
    );
    assert!(chain.method(&crc_key()).is_none());
    let run = Command::new(f.home.join("bin/java"))
        .args(["-Xverify:all", "-cp", ".", "DefaultFixture"])
        .current_dir(&f.path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "CRC32->Checksum.update([B)V;26 CRC cases;NPE default frame;checked sentinel same;effects=RRM;class dominance"
    );
    fs::write(f.path.join("runtime.txt"), &run.stdout).unwrap();
    let javap = Command::new(f.home.join("bin/javap"))
        .args(["-p", "-c", "-s", "DefaultFixture"])
        .current_dir(&f.path)
        .output()
        .unwrap();
    assert!(javap.status.success());
    let source = String::from_utf8_lossy(&javap.stdout);
    assert!(source.contains("java/util/zip/CRC32.update:([B)V"));
    assert!(source.contains("DefaultFixture$Diamond.apply:([B)I"));
    fs::write(f.path.join("javap.txt"), javap.stdout).unwrap();
}
#[test]
fn single_origin_diamond_most_specific_default_and_checked_throws() {
    let f = fixture();
    let diamond =
        resolve_inherited_public_instance(&f.cp, &key("LDefaultFixture$Diamond;")).unwrap();
    assert_eq!(diamond.declaration().key.owner, "LDefaultFixture$Root;");
    assert_eq!(
        diamond.declaration().metadata.exceptions,
        Some(vec!["Ljava/io/IOException;".into()])
    );
    let most = resolve_inherited_public_instance(&f.cp, &key("LDefaultFixture$Most;")).unwrap();
    assert_eq!(most.declaration().key.owner, "LDefaultFixture$More;");
    assert!(matches!(
        resolve_inherited_public_instance(&f.cp, &key("LDefaultFixture$Own;")),
        Err(InheritedMethodError::ClassDominance(_))
    ));
    let inherited_class =
        resolve_inherited_public_instance(&f.cp, &key("LDefaultFixture$OwnChild;")).unwrap();
    assert_eq!(inherited_class.kind(), InheritedMethodKind::ClassInstance);
    assert_eq!(
        inherited_class.declaration().key.owner,
        "LDefaultFixture$Own;"
    );
    for owner in [
        "LDefaultFixture$AbstractClass;",
        "LDefaultFixture$PrivateClass;",
        "LDefaultFixture$StaticClass;",
    ] {
        assert!(matches!(
            resolve_inherited_public_instance(&f.cp, &key(owner)),
            Err(InheritedMethodError::NoMethod(_))
        ));
    }
    assert!(matches!(
        resolve_inherited_public_instance(&f.cp, &key("LDefaultFixture$GenericClass;")),
        Err(InheritedMethodError::Generic(_))
    ));
}
// Deliberately malformed/uncertain metadata tests: these cloned genuine boundary
// declarations are adversarial evidence, never offered as real API declarations.
struct Snapshot(BTreeMap<String, ClassMetadata>);
impl InheritedMethodMetadata for Snapshot {
    fn default_class(&self, owner: &str) -> Option<&ClassMetadata> {
        self.0.get(owner)
    }
}
fn snapshot() -> Snapshot {
    Snapshot(
        fixture()
            .cp
            .classes()
            .filter(|c| {
                c.descriptor.starts_with("LDefaultFixture") || c.descriptor == "Ljava/lang/Object;"
            })
            .map(|c| (c.descriptor.clone(), c.clone()))
            .collect(),
    )
}
#[test]
fn missing_cycles_ambiguous_abstract_diamonds_and_invalid_declarations_deny() {
    for mode in 0..19 {
        let mut md = snapshot();
        let root = "LDefaultFixture$Root;";
        let left = "LDefaultFixture$Left;";
        let right = "LDefaultFixture$Right;";
        let diamond = "LDefaultFixture$Diamond;";
        match mode {
            0 => {
                md.0.remove("Ljava/lang/Object;");
            }
            1 => {
                md.0.remove(left);
            }
            2 => {
                md.0.get_mut(root)
                    .unwrap()
                    .declaration
                    .interfaces
                    .push("DefaultFixture$Left".into())
            }
            3 => {
                let competing = md.0[root]
                    .methods
                    .iter()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .clone();
                for owner in [left, right] {
                    let mut method = competing.clone();
                    method.key.owner = owner.into();
                    md.0.get_mut(owner).unwrap().methods.push(method);
                }
            }
            4 => {
                md.0.get_mut(diamond)
                    .unwrap()
                    .declaration
                    .interfaces
                    .push("DefaultFixture$UnrelatedAbstract".into())
            }
            5 => md.0.get_mut(root).unwrap().access_flags &= !1,
            6 => {
                md.0.get_mut(root)
                    .unwrap()
                    .methods
                    .iter_mut()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .metadata
                    .access_flags = 2
            }
            7 => {
                md.0.get_mut(root)
                    .unwrap()
                    .methods
                    .iter_mut()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .metadata
                    .access_flags = 9
            }
            8 => {
                md.0.get_mut(root)
                    .unwrap()
                    .methods
                    .iter_mut()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .metadata
                    .exceptions = None
            }
            9 => {
                md.0.get_mut(root)
                    .unwrap()
                    .methods
                    .iter_mut()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .code = None
            }
            10 => {
                let m = md.0[root]
                    .methods
                    .iter()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .clone();
                md.0.get_mut(root).unwrap().methods.push(m);
            }
            11 => {
                md.0.get_mut(diamond).unwrap().declaration.superclass =
                    Some("DefaultFixture$Root".into())
            }
            12 => {
                md.0.get_mut(diamond)
                    .unwrap()
                    .declaration
                    .interfaces
                    .push("DefaultFixture$Own".into())
            }
            13 => {
                md.0.get_mut(root)
                    .unwrap()
                    .methods
                    .iter_mut()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .metadata
                    .access_flags = 0x101
            }
            14 => {
                md.0.get_mut(root)
                    .unwrap()
                    .methods
                    .iter_mut()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .signature = Some("([B)I".into())
            }
            15..=18 => {
                let flag = match mode {
                    15 => 0x11,
                    16 => 0x21,
                    17 => 0x1041,
                    _ => 0x1001,
                };
                md.0.get_mut(root)
                    .unwrap()
                    .methods
                    .iter_mut()
                    .find(|m| m.key.name == "apply")
                    .unwrap()
                    .metadata
                    .access_flags = flag;
            }
            _ => unreachable!(),
        }
        assert!(
            resolve_inherited_public_instance(&md, &key(diamond)).is_err(),
            "mode{mode}"
        );
    }
    let mut constructor = crc_key();
    constructor.name = "<init>".into();
    constructor.parameters.clear();
    assert!(resolve_inherited_public_instance(&fixture().cp, &constructor).is_err());
    assert!(
        resolve_inherited_public_instance(&fixture().cp, &key("LDefaultFixture$Root;")).is_err(),
        "interface owner is outside class-reference capability"
    );
    let mut wrong_return = crc_key();
    wrong_return.return_type = "I".into();
    assert!(resolve_inherited_public_instance(&fixture().cp, &wrong_return).is_err());
}

#[test]
fn original_samsung_135_137_158_call_chain_retains_crc32_symbolic_identity() {
    use std::io::Read;
    let apk = "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk";
    let mut archive = zip::ZipArchive::new(fs::File::open(apk).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("classes.dex")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let dex = dex_format::DexFile::from_bytes(&bytes).unwrap();
    let descriptor = |index| {
        dex.string(dex.type_id(index).unwrap().descriptor_idx)
            .unwrap()
    };
    let method_key = |index| {
        let raw = dex.method_id(index).unwrap();
        let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
        MethodKey {
            owner: descriptor(u32::from(raw.class_idx)).into(),
            name: dex.string(raw.name_idx).unwrap().into(),
            parameters: proto
                .parameters
                .as_ref()
                .into_iter()
                .flat_map(|p| &p.types)
                .map(|i| descriptor(u32::from(*i)).into())
                .collect(),
            return_type: descriptor(proto.return_type_idx).into(),
        }
    };
    assert_eq!(method_key(47752), crc_key());
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(c.class_idx) == "La4/a;")
        .unwrap();
    let data = class.class_data.as_ref().unwrap();
    let mut log = String::new();
    for (index, reference, offsets) in [
        (135, 137, vec![102]),
        (137, 158, vec![122]),
        (158, 47752, vec![108, 111]),
    ] {
        let method = data
            .direct_methods
            .iter()
            .chain(&data.virtual_methods)
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        fs::write(
            format!("/tmp/opencode/wave12-external-default/original{index}.insns"),
            &code.insns,
        )
        .unwrap();
        let calls=dex_decompiler::decode_method(code).unwrap().into_iter().filter(|i|i.pool_index==Some(reference) && matches!(i.opcode,dex_decompiler::Opcode::Supported(op) if (0x6e..=0x72).contains(&op.opcode))).collect::<Vec<_>>();
        assert_eq!(calls.iter().map(|i| i.offset).collect::<Vec<_>>(), offsets);
        log += &format!(
            "original pool0 #{index} {:?};flags={:#x};calls={calls:?};target={:?};tries={:?};handlers={:?}\n",
            method_key(index),
            method.access_flags,
            method_key(reference),
            code.tries,
            code.handlers
        );
    }
    let proof = resolve_inherited_public_instance(&fixture().cp, &method_key(47752)).unwrap();
    log += &format!(
        "selected {:?};flags={:#x};throws={:?};owners={:?};code={:?}\n",
        proof.declaration().key,
        proof.declaration().metadata.access_flags,
        proof.declaration().metadata.exceptions,
        proof.owners(),
        proof.declaration().code.as_ref().unwrap().bytes
    );
    fs::write(
        "/tmp/opencode/wave12-external-default/original-chain.txt",
        log,
    )
    .unwrap();
}

#[test]
fn genuine_android35_inherited_view_method_compiles_with_original_symbolic_owner() {
    let f = fixture();
    let sdk =
        Classpath::from_paths(["/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"])
            .unwrap();
    let chain = MetadataChain::new(vec![
        MetadataProvider::Jvm(&f.cp),
        MetadataProvider::Jvm(&sdk),
    ]);
    let reference = MethodKey {
        owner: "Landroid/view/ViewGroup;".into(),
        name: "setHandwritingBoundsOffsets".into(),
        parameters: vec!["F".into(); 4],
        return_type: "V".into(),
    };
    assert!(chain.method(&reference).is_none());
    let proof = resolve_inherited_public_instance(&chain, &reference).unwrap();
    assert_eq!(proof.kind(), InheritedMethodKind::ClassInstance);
    assert_eq!(proof.declaration().key.owner, "Landroid/view/View;");
    assert_eq!(proof.declaration().metadata.access_flags, 1);
    assert_eq!(proof.declaration().metadata.exceptions, Some(vec![]));
    assert!(
        proof.declaration().code.is_some(),
        "genuine SDK declaration body, not invented source"
    );
    let path = f.path.join("android-proof");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("ViewOracle.java"),"public class ViewOracle { public static void original(android.view.ViewGroup group,float a,float b,float c,float d) { group.setHandwritingBoundsOffsets(a,b,c,d); } }\n").unwrap();
    let compile = Command::new(f.home.join("bin/javac"))
        .args([
            "--release",
            "17",
            "-cp",
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
            "ViewOracle.java",
        ])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let javap = Command::new(f.home.join("bin/javap"))
        .args(["-p", "-c", "-s", "ViewOracle"])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(javap.status.success());
    let text = String::from_utf8_lossy(&javap.stdout);
    assert!(text.contains("android/view/ViewGroup.setHandwritingBoundsOffsets:(FFFF)V"));
    fs::write(path.join("javap.txt"), javap.stdout).unwrap();
    fs::write(path.join("proof.txt"),format!("reference={:?};selected={:?};flags={:#x};throws={:?};owners={:?};SDK declaration only, Android runtime not executed\n",proof.reference(),proof.declaration().key,proof.declaration().metadata.access_flags,proof.declaration().metadata.exceptions,proof.owners())).unwrap();
}

#[test]
fn class_native_abstract_contract_and_concrete_override_identity_are_preserved() {
    let f = fixture();
    let path = f.path.join("class-proof");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("InheritedClassFixture.java"),r#"
import java.io.IOException;
public class InheritedClassFixture {
  static String events = "";
  static IOException fault = new IOException("same-object");
  public interface Default { default int apply(byte[] b) throws IOException { events += "D"; return -2; } }
  public static class Parent { public int apply(byte[] b) throws IOException { events += "P"; if (b == null) throw fault; return b.length; } }
  public static class Child extends Parent {}
  public static class Override extends Child { public int apply(byte[] b) throws IOException { events += "O"; if (b == null) throw fault; return b.length + 1; } }
  public static abstract class AbstractParent { public abstract int apply(byte[] b) throws IOException; }
  public static abstract class AbstractChild extends AbstractParent {}
  public static class AbstractImplementation extends AbstractChild { public int apply(byte[] b) { events += "A"; return b.length; } }
  public static class NativeParent { public native int apply(byte[] b) throws IOException; }
  public static class NativeChild extends NativeParent {}
  public static class DominantParent { private int apply(byte[] b) { return 0; } }
  public static class PrivateChild extends DominantParent {}
  public static class StaticParent { public static int apply(byte[] b) { return 0; } }
  public static class StaticChild extends StaticParent {}
  public static abstract class AbstractDominance extends AbstractParent implements Default {}
  static int call(Child receiver,byte[] b) throws IOException { return receiver.apply(b); }
  static int abstractCall(AbstractChild receiver,byte[] b) throws IOException { return receiver.apply(b); }
  static int nativeCall(NativeChild receiver,byte[] b) throws IOException { return receiver.apply(b); }
  public static void main(String[] args) throws Exception {
    if (call(new Child(),new byte[2]) != 2 || call(new Override(),new byte[2]) != 3) throw new AssertionError();
    try { call(new Override(),null); throw new AssertionError(); }
    catch (IOException e) { if (e != fault || !events.equals("POO")) throw new AssertionError("dynamic identity/effects"); }
    if (abstractCall(new AbstractImplementation(),new byte[1]) != 1 || !events.equals("POOA")) throw new AssertionError("abstract dispatch");
    try { nativeCall(new NativeChild(),new byte[1]); throw new AssertionError(); }
    catch (UnsatisfiedLinkError e) { if (!e.getMessage().contains("InheritedClassFixture$NativeParent.apply")) throw new AssertionError("native unchanged"); }
    System.out.println("class effects=POOA;same checked sentinel;abstract actual implementation;native linkage unchanged");
  }
}
"#).unwrap();
    let compile = Command::new(f.home.join("bin/javac"))
        .args(["--release", "17", "InheritedClassFixture.java"])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let mut cp = Classpath::default();
    cp.add_class(&fs::read(path.join("InheritedClassFixture.class")).unwrap())
        .unwrap();
    for entry in fs::read_dir(&path).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "class")
            && p.file_name().unwrap() != "InheritedClassFixture.class"
        {
            cp.add_path(p).unwrap();
        }
    }
    let chain = MetadataChain::new(vec![
        MetadataProvider::Jvm(&cp),
        MetadataProvider::Jvm(&f.cp),
    ]);
    for (owner, selected, flags) in [
        ("Child", "Parent", 1),
        ("AbstractChild", "AbstractParent", 0x401),
        ("NativeChild", "NativeParent", 0x101),
        ("AbstractDominance", "AbstractParent", 0x401),
    ] {
        let proof = resolve_inherited_public_instance(
            &chain,
            &key(&format!("LInheritedClassFixture${owner};")),
        )
        .unwrap();
        assert_eq!(proof.kind(), InheritedMethodKind::ClassInstance);
        assert_eq!(
            proof.declaration().key.owner,
            format!("LInheritedClassFixture${selected};")
        );
        assert_eq!(proof.declaration().metadata.access_flags, flags);
        assert_eq!(
            proof.declaration().metadata.exceptions,
            Some(vec!["Ljava/io/IOException;".into()])
        );
        assert_eq!(proof.declaration().code.is_none(), flags != 1);
    }
    for owner in ["PrivateChild", "StaticChild"] {
        assert!(
            resolve_inherited_public_instance(
                &chain,
                &key(&format!("LInheritedClassFixture${owner};"))
            )
            .is_err()
        );
    }
    let run = Command::new(f.home.join("bin/java"))
        .args(["-Xverify:all", "-cp", ".", "InheritedClassFixture"])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "class effects=POOA;same checked sentinel;abstract actual implementation;native linkage unchanged"
    );
    fs::write(path.join("runtime.txt"), run.stdout).unwrap();
}

#[test]
fn original_search_bridge_32783_source_is_accepted_and_compiles_unchanged() {
    use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
    use dex_decompiler::infer::TypeContext;
    use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(
        fs::File::open(
            "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
        )
        .unwrap(),
    )
    .unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("classes.dex")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let dex = dex_format::DexFile::from_bytes(&bytes).unwrap();
    let descriptor = |i| dex.string(dex.type_id(i).unwrap().descriptor_idx).unwrap();
    let key = crate::java_external_metadata::resolve_method;
    let _ = key; // Exact declared API remains separate from the inherited capability.
    let caller = crate::java_members::source_closure::method_key(&dex, 32783).unwrap();
    assert_eq!(caller.owner, "Lcom/google/android/material/search/a;");
    assert_eq!(caller.name, "a");
    assert_eq!(caller.return_type, "V");
    let reference = crate::java_members::source_closure::method_key(&dex, 4066).unwrap();
    assert_eq!(reference.owner, "Landroid/view/ViewGroup;");
    assert_eq!(reference.name, "setHandwritingBoundsOffsets");
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(c.class_idx) == caller.owner)
        .unwrap();
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 32783)
        .unwrap();
    assert_eq!(method.access_flags, 0x1049);
    let code = method.code.as_ref().unwrap();
    fs::write(
        "/tmp/opencode/wave12-external-default/original32783.insns",
        &code.insns,
    )
    .unwrap();
    let decoded = dex_decompiler::decode_method(code).unwrap();
    assert_eq!(decoded[0].offset, 0);
    assert_eq!(decoded[0].pool_index, Some(4066));
    assert!(matches!(decoded[0].opcode,dex_decompiler::Opcode::Supported(op) if op.opcode==0x6e));
    let f = fixture();
    let sdk =
        Classpath::from_paths(["/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"])
            .unwrap();
    let cp = MetadataChain::new(vec![
        MetadataProvider::Jvm(&f.cp),
        MetadataProvider::Jvm(&sdk),
    ]);
    let hierarchy = crate::java_hierarchy::build_hierarchy(&[&dex], cp.hierarchy()).unwrap();
    struct R<'a> {
        dex: &'a dex_format::DexFile,
        cp: &'a MetadataChain<'a>,
        h: &'a backend_java::type_compatibility::Hierarchy,
        proof: bool,
    }
    impl JavaResolver for R<'_> {
        fn class(&self, r: ClassRef) -> Option<String> {
            let d = self
                .dex
                .string(self.dex.type_id(r.index()).ok()?.descriptor_idx)
                .ok()?;
            Some(
                d.trim_start_matches('L')
                    .trim_end_matches(';')
                    .replace('/', "."),
            )
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            let key = crate::java_members::source_closure::method_key(self.dex, r.index())?;
            let metadata = self.cp.method(&key).or_else(|| {
                self.proof
                    .then(|| resolve_inherited_public_instance(self.cp, &key).ok())
                    .flatten()
                    .map(|p| p.declaration().metadata.clone())
            })?;
            Some(JavaMethod {
                owner: key
                    .owner
                    .trim_start_matches('L')
                    .trim_end_matches(';')
                    .replace('/', "."),
                name: key.name,
                is_static: metadata.access_flags & 8 != 0,
                parameters: key
                    .parameters
                    .iter()
                    .map(|d| crate::dex_java::descriptor_type(d))
                    .collect::<Option<_>>()?,
                return_type: crate::dex_java::descriptor_type(&key.return_type)?,
                throws: metadata
                    .exceptions?
                    .iter()
                    .map(|d| {
                        d.trim_start_matches('L')
                            .trim_end_matches(';')
                            .replace('/', ".")
                    })
                    .collect(),
            })
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            self.h.assignable(a, b) == backend_java::type_compatibility::Compatibility::Assignable
        }
    }
    let context = dex_decompiler::DexTypes::new(&dex, Some(32783));
    let mut types = Vec::new();
    assert!(context.method_params(32783, &mut types));
    let mut reg = code.registers_size - code.ins_size;
    let mut seeds = std::collections::HashMap::new();
    for ty in types {
        let wide = matches!(
            ty,
            managed_ir::types::ManagedType::Long | managed_ir::types::ManagedType::Double
        );
        seeds.insert((reg, wide), ty);
        reg += if wide { 2 } else { 1 };
    }
    assert_eq!(reg, code.registers_size);
    let ssa = dex_decompiler::DexPasses::run_ssa_with(code, &context.with_initial_registers(seeds))
        .unwrap();
    let signature = JavaMethod {
        owner: caller
            .owner
            .trim_start_matches('L')
            .trim_end_matches(';')
            .replace('/', "."),
        name: caller.name.clone(),
        is_static: true,
        parameters: caller
            .parameters
            .iter()
            .map(|d| crate::dex_java::descriptor_type(d))
            .collect::<Option<_>>()
            .unwrap(),
        return_type: JavaType::Void,
        throws: vec![],
    };
    let provenance = ssa
        .initial_registers()
        .iter()
        .map(
            |(value, registers)| crate::java_initial_parameters::InitialProvenance {
                value: *value,
                registers: registers.clone(),
            },
        )
        .collect::<Vec<_>>();
    let plan = crate::java_initial_parameters::parameter_bindings_with_provenance(
        ssa.ir(),
        &signature,
        code.registers_size,
        code.ins_size,
        &provenance,
    )
    .unwrap();
    let bound = |p| match p {
        crate::java_initial_parameters::InitialParameter::Bound(v) => Some(v),
        crate::java_initial_parameters::InitialParameter::Unused { .. } => None,
    };
    let bindings = backend_java::DeclarationBindings {
        receiver: None,
        parameters: plan.parameters.into_iter().map(bound).collect(),
        ..Default::default()
    };
    let before = backend_java::emit_method_declaration(
        ssa.ir(),
        &signature,
        &bindings,
        &R {
            dex: &dex,
            cp: &cp,
            h: &hierarchy,
            proof: false,
        },
    );
    assert!(matches!(before, backend_java::SourceResult::Unsupported(_)));
    let after = backend_java::emit_method_declaration(
        ssa.ir(),
        &signature,
        &bindings,
        &R {
            dex: &dex,
            cp: &cp,
            h: &hierarchy,
            proof: true,
        },
    );
    let backend_java::SourceResult::Complete { source, .. } = after else {
        panic!("{after:?}")
    };
    let path = f.path.join("original32783");
    fs::create_dir_all(path.join("com/google/android/material/search")).unwrap();
    let java = path.join("com/google/android/material/search/a.java");
    fs::write(
        &java,
        format!("package com.google.android.material.search;\npublic class a {{\n{source}\n}}\n"),
    )
    .unwrap();
    fs::write(path.join("original-emitted-method.txt"), &source).unwrap();
    // Method-free boundary classfiles retain genuine relevant superclass edges;
    // no peer constructor or method body is fabricated as an ORIGINAL method.
    let search = dex
        .classes
        .iter()
        .find(|c| descriptor(c.class_idx) == "Lcom/google/android/material/search/SearchBar;")
        .unwrap();
    let toolbar = dex
        .classes
        .iter()
        .find(|c| descriptor(c.class_idx) == "Landroidx/appcompat/widget/Toolbar;")
        .unwrap();
    assert_eq!(
        descriptor(search.superclass_idx),
        "Landroidx/appcompat/widget/Toolbar;"
    );
    assert_eq!(
        descriptor(toolbar.superclass_idx),
        "Landroid/view/ViewGroup;"
    );
    fs::write(path.join("HeaderBoundaries.java"),r#"
import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;
public class HeaderBoundaries {
 public static void main(String[] args)throws Exception {
  String[][] edges={{"androidx.appcompat.widget.Toolbar","android.view.ViewGroup"},{"com.google.android.material.search.SearchBar","androidx.appcompat.widget.Toolbar"}};
  for(String[] e:edges){byte[] b=ClassFile.of().build(ClassDesc.of(e[0]),c->{c.withVersion(61,0);c.withFlags(ClassFile.ACC_PUBLIC|ClassFile.ACC_ABSTRACT|ClassFile.ACC_SUPER);c.withSuperclass(ClassDesc.of(e[1]));});Path p=Path.of(e[0].replace('.','/')+".class");Files.createDirectories(p.getParent());Files.write(p,b);}
 }
}
"#).unwrap();
    let jdk25 = PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin");
    for (tool, args) in [
        ("javac", vec!["HeaderBoundaries.java"]),
        ("java", vec!["HeaderBoundaries"]),
    ] {
        let out = Command::new(jdk25.join(tool))
            .args(args)
            .current_dir(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let compile = Command::new(f.home.join("bin/javac"))
        .args([
            "--release",
            "17",
            "-cp",
            ".:/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
            "com/google/android/material/search/a.java",
        ])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(
        fs::read_to_string(&java).unwrap().contains(&source),
        "emitted method unchanged"
    );
    let javap = Command::new(f.home.join("bin/javap"))
        .args(["-p", "-c", "-s", "com.google.android.material.search.a"])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(javap.status.success());
    assert!(
        String::from_utf8_lossy(&javap.stdout)
            .contains("android/view/ViewGroup.setHandwritingBoundsOffsets:(FFFF)V")
    );
    fs::write(path.join("javap.txt"), javap.stdout).unwrap();
    fs::write(path.join("original-proof.txt"),format!("original pool0#32783 {caller:?};flags={:#x};words={};raw={decoded:?};before={before:?};actual backend source complete unchanged;method-free peer header boundaries only;no Android runtime\n",method.access_flags,code.insns_size)).unwrap();
}
