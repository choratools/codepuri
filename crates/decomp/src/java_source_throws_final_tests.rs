#![allow(clippy::unwrap_used)]
use super::*;

type Fixture = (
    Vec<DexFile>,
    JavaAliases,
    BTreeMap<MethodKey, Option<MethodMetadata>>,
    crate::java_external_metadata::Classpath,
    crate::type_compatibility::Hierarchy,
);
fn fixture() -> Fixture {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let external = crate::java_external_metadata::Classpath::from_paths(&[
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
        std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let mut inventory = AliasInventory::default();
    for dex in &files {
        for ty in &dex.type_ids {
            inventory
                .add_type(dex.string(ty.descriptor_idx).unwrap())
                .unwrap();
        }
        for index in 0..dex.method_ids.len() {
            inventory
                .add_method(method_key(dex, index as u32).unwrap())
                .unwrap();
        }
        for index in 0..dex.field_ids.len() {
            inventory
                .add_field(field_key(dex, index as u32).unwrap())
                .unwrap();
        }
    }
    let aliases = inventory.build(AliasMode::Ascii);
    let hierarchy = crate::java_hierarchy::build_hierarchy(
        &files.iter().collect::<Vec<_>>(),
        external.hierarchy(),
    )
    .unwrap()
    .map_names(|n| {
        aliases
            .class_name(&format!("L{};", n.replace('.', "/")))
            .unwrap_or(n)
            .into()
    })
    .unwrap();
    let catalog = method_catalog(&files);
    (files, aliases, catalog, external, hierarchy)
}

#[test]
#[ignore = "genuine Samsung final InputStream contracts with JDK17/Android35"]
fn samsung_final_input_stream_contracts() {
    let (files, aliases, catalog, external, hierarchy) = fixture();
    let indices = [590, 591, 594, 595, 596, 597, 598];
    let roots = indices
        .into_iter()
        .map(|i| method_key(&files[0], i).unwrap())
        .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    for index in indices {
        let key = method_key(&files[0], index).unwrap();
        println!(
            "FINAL {index} source={:?} unavailable={:?}",
            plan.methods.get(&key),
            plan.unavailable.get(&key)
        );
        let method = plan.methods.get(&key).unwrap();
        assert_eq!(method.source, vec!["java.io.IOException"]);
    }
}
#[test]
#[ignore = "real parsed final class metadata mutation boundaries"]
fn final_source_exclusion_preserves_closed_gates() {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let dex = &files[0];
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == Some("Laf/a;"))
        .unwrap();
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .virtual_methods
        .iter()
        .find(|m| m.method_idx == 590)
        .unwrap();
    assert_eq!(class.annotations_off, 0);
    assert!(!concrete_source_reconstructible(dex, class, method));
    let mut final_method = method.clone();
    final_method.access_flags |= 0x10;
    assert!(!concrete_source_reconstructible(dex, class, &final_method));
    for flag in [2, 8, 0x100, 0x400] {
        let mut m = method.clone();
        m.access_flags |= flag;
        assert!(!concrete_source_reconstructible(dex, class, &m));
    }
    let mut c = class.clone();
    c.annotations_off = 1;
    assert!(!concrete_source_reconstructible(dex, &c, method));
    let mut c = class.clone();
    c.class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .push(method.clone());
    assert!(!concrete_source_reconstructible(dex, &c, method));
    let original = dex
        .classes
        .iter()
        .find(|c| {
            c.annotations.as_ref().is_some_and(|d| {
                d.method_annotations.iter().any(|m| {
                    m.annotations.entries.iter().any(|a| {
                        descriptor(dex, a.annotation.type_idx) == Some("Ldalvik/annotation/Throws;")
                    })
                })
            })
        })
        .unwrap();
    let mut c = class.clone();
    let mut directory = original.annotations.clone().unwrap();
    let mut member = directory
        .method_annotations
        .iter()
        .find(|m| {
            m.annotations.entries.iter().any(|a| {
                descriptor(dex, a.annotation.type_idx) == Some("Ldalvik/annotation/Throws;")
            })
        })
        .unwrap()
        .clone();
    member.member_idx = 590;
    for item in &mut member.annotations.entries {
        if descriptor(dex, item.annotation.type_idx) == Some("Ldalvik/annotation/Throws;") {
            assert!(!item.annotation.elements.is_empty());
            for element in &mut item.annotation.elements {
                element.value.value = dex_format::encoded_value::EncodedValueKind::Array(vec![]);
            }
        }
    }
    directory.method_annotations = vec![member];
    c.annotations = Some(directory);
    c.annotations_off = 1;
    assert!(!concrete_source_reconstructible(dex, &c, method));
}

#[test]
fn final_override_rethrow_preserves_identity_and_telemetry_order() {
    let dir = std::env::temp_dir().join(format!("decomphify-final-throws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = r#"import java.io.*;
public class FinalContract {
 static String events=""; static IOException failure=new IOException("original"); static RuntimeException telemetry=new RuntimeException("telemetry"); static boolean replace;
 static class Boundary extends InputStream {
  void invoke()throws IOException {events+="I";throw failure;}
  public int available()throws IOException{invoke();return 0;}
  public void close()throws IOException{invoke();}
  public int read()throws IOException{invoke();return 0;}
  public int read(byte[]b)throws IOException{invoke();return 0;}
  public int read(byte[]b,int o,int n)throws IOException{invoke();return 0;}
  public void reset()throws IOException{invoke();}
  public long skip(long n)throws IOException{invoke();return 0;}
 }
 static final class Wrapper extends InputStream {
  final InputStream input=new Boundary();
  void report(){events+="T1";if(replace)throw telemetry;events+="T2";}
  public int available()throws IOException{try{return input.available();}catch(IOException e){report();throw e;}}
  public void close()throws IOException{try{input.close();}catch(IOException e){report();throw e;}}
  public int read()throws IOException{try{return input.read();}catch(IOException e){report();throw e;}}
  public int read(byte[]b)throws IOException{try{return input.read(b);}catch(IOException e){report();throw e;}}
  public int read(byte[]b,int o,int n)throws IOException{try{return input.read(b,o,n);}catch(IOException e){report();throw e;}}
  public void reset()throws IOException{try{input.reset();}catch(IOException e){report();throw e;}}
  public long skip(long n)throws IOException{try{return input.skip(n);}catch(IOException e){report();throw e;}}
 }
 public static void main(String[]args)throws Exception{for(int k=0;k<7;k++)for(int r=0;r<2;r++){events="";replace=r==1;Wrapper w=new Wrapper();Throwable caught=null;try{switch(k){case 0:w.available();break;case 1:w.close();break;case 2:w.read();break;case 3:w.read(new byte[1]);break;case 4:w.read(new byte[1],0,1);break;case 5:w.reset();break;case 6:w.skip(1);break;}}catch(Throwable e){caught=e;}if(caught!=(replace?telemetry:failure)||!events.equals(replace?"IT1":"IT1T2"))throw new AssertionError(k+":"+events);}System.out.println("14 identity/order/replacement cases");}
}"#;
    std::fs::write(dir.join("FinalContract.java"), source).unwrap();
    let jdk = std::path::Path::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin");
    let out = std::process::Command::new(jdk.join("javac"))
        .arg(dir.join("FinalContract.java"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new(jdk.join("java"))
        .arg("-cp")
        .arg(&dir)
        .arg("FinalContract")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("14 identity/order/replacement cases"));
    std::fs::write(dir.join("IllegalBound.java"),"class Parent { void call(){} } final class IllegalBound extends Parent { void call()throws java.io.IOException{} }").unwrap();
    let out = std::process::Command::new(jdk.join("javac"))
        .arg(dir.join("IllegalBound.java"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not throw IOException"));
}
