//! Original private-final helper bodies, independently checked against clinit order.
#![allow(clippy::unwrap_used)]
use super::*;
use std::collections::HashSet;

#[test]
#[ignore = "six original Samsung enum helpers, local APK, no full export"]
fn samsung_private_final_helpers_preserve_original_array_order() {
    let inputs = dex_format::apk::parse_apk_path(
        "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let owners = [
        "Lcom/samsung/android/samsungpass/backupandrestore/OSmosisStatus;",
        "Lcom/samsung/android/samsungpass/certificate/model/AuthenticationResult$AuthenticationType;",
        "Lcom/samsung/android/samsungpass/certificate/ui/handler/CertificateErrorHandler$ErrorSeverity;",
        "Lcom/samsung/android/samsungpass/certificate/ui/handler/CertificateProgressHandler$ProgressType;",
        "Lcom/samsung/android/samsungpass/common/browser/ChromeAutofillManager$ChromeBrowserType;",
        "Lcom/samsung/android/samsungpass/intro/BiometricState;",
    ];
    let mut proved = 0;
    for owner in owners {
        let (dex, class) = inputs
            .iter()
            .find_map(|dex| {
                dex.classes
                    .iter()
                    .find(|class| desc(dex, class.class_idx) == Some(owner))
                    .map(|class| (dex, class))
            })
            .unwrap();
        let data = class.class_data.as_ref().unwrap();
        let enum_fields: HashSet<_> = data
            .static_fields
            .iter()
            .filter(|field| field.access_flags == 0x4019)
            .map(|field| field.field_idx)
            .collect();
        let clinit = data
            .direct_methods
            .iter()
            .find(|method| {
                dex.string(dex.method_id(method.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "<clinit>"
            })
            .unwrap();
        let constants: Vec<_> = dex_decompiler::decode_method(clinit.code.as_ref().unwrap())
            .unwrap()
            .into_iter()
            .filter_map(|insn| {
                let dex_decompiler::Opcode::Supported(info) = insn.opcode else {
                    return None;
                };
                if info.name != "sput-object" {
                    return None;
                }
                insn.pool_index.filter(|field| enum_fields.contains(field))
            })
            .enumerate()
            .map(|(ordinal, field_idx)| super::super::EnumConstant {
                field_idx,
                name: dex
                    .string(dex.field_id(field_idx).unwrap().name_idx)
                    .unwrap()
                    .into(),
                ordinal,
                arguments: vec![],
            })
            .collect();
        assert_eq!(constants.len(), enum_fields.len());
        let helper = data
            .direct_methods
            .iter()
            .find(|method| {
                dex.string(dex.method_id(method.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "$values"
            })
            .unwrap();
        assert_eq!(helper.access_flags, 0x101a);
        assert!(prove_values_helper(
            dex,
            class,
            helper.method_idx,
            &constants
        ));
        let mut reordered = constants.clone();
        reordered.reverse();
        assert!(!prove_values_helper(
            dex,
            class,
            helper.method_idx,
            &reordered
        ));
        for flags in [0x1019, 0x101c, 0x1018, 0x111a, 0x103a] {
            let mut changed = class.clone();
            changed
                .class_data
                .as_mut()
                .unwrap()
                .direct_methods
                .iter_mut()
                .find(|method| method.method_idx == helper.method_idx)
                .unwrap()
                .access_flags = flags;
            assert!(!prove_values_helper(
                dex,
                &changed,
                helper.method_idx,
                &constants
            ));
        }
        let mut changed = class.clone();
        changed
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|method| method.method_idx == helper.method_idx)
            .unwrap()
            .code
            .as_mut()
            .unwrap()
            .insns[2] ^= 1;
        assert!(!prove_values_helper(
            dex,
            &changed,
            helper.method_idx,
            &constants
        ));
        eprintln!(
            "{owner} helper={} flags=0x101a original_order={constants:?}",
            helper.method_idx
        );
        proved += 1;
    }
    assert_eq!(proved, 6);
}

#[test]
fn private_final_array_helper_jvm_source_differential() {
    use std::process::Command;
    let root = std::env::temp_dir().join(format!("decomphify-enum-helper-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    // Exact six-owner witness above establishes this array-only helper shape;
    // this execution oracle covers allocation identity and constant order. It
    // is deliberately a boundary fixture, not a whole Samsung-class claim.
    std::fs::write(
        root.join("HelperEnum.java"),
        r#"
public enum HelperEnum {
    STARTED, PROGRESS, SUCCESS, ERROR;
    private static final HelperEnum[] originalSource() {
        return new HelperEnum[]{STARTED, PROGRESS, SUCCESS, ERROR};
    }
    public static HelperEnum[] source() { return originalSource(); }
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("MakeOracle.java"),
        r#"
import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
public class MakeOracle implements Opcodes {
    public static void main(String[] args) throws Exception {
        ClassWriter w = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        w.visit(V17, ACC_PUBLIC, "RawHelper", null, "java/lang/Object", null);
        MethodVisitor m = w.visitMethod(ACC_PUBLIC|ACC_STATIC|ACC_FINAL|ACC_SYNTHETIC,
            "array", "()[LHelperEnum;", null, null);
        m.visitCode(); m.visitInsn(ICONST_4); m.visitTypeInsn(ANEWARRAY,"HelperEnum");
        String[] names = {"STARTED","PROGRESS","SUCCESS","ERROR"};
        for(int i=0;i<names.length;i++) {
            m.visitInsn(DUP);m.visitInsn(ICONST_0+i);
            m.visitFieldInsn(GETSTATIC,"HelperEnum",names[i],"LHelperEnum;");
            m.visitInsn(AASTORE);
        }
        m.visitInsn(ARETURN);m.visitMaxs(0,0);m.visitEnd();w.visitEnd();
        Files.write(Path.of("RawHelper.class"), w.toByteArray());
        ClassWriter e = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        e.visit(V17, ACC_PUBLIC|ACC_FINAL|ACC_ENUM,"RawEnum",null,"java/lang/Enum",null);
        MethodVisitor c=e.visitMethod(ACC_PUBLIC,"<init>","(Ljava/lang/String;I)V",null,null);
        c.visitCode();c.visitVarInsn(ALOAD,0);c.visitVarInsn(ALOAD,1);c.visitVarInsn(ILOAD,2);
        c.visitMethodInsn(INVOKESPECIAL,"java/lang/Enum","<init>","(Ljava/lang/String;I)V",false);
        c.visitInsn(RETURN);c.visitMaxs(0,0);c.visitEnd();e.visitEnd();
        Files.write(Path.of("RawEnum.class"),e.toByteArray());
    }
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("Check.java"),
        r#"
import java.util.Arrays;
public class Check {
    private static HelperEnum[] raw() throws Exception {
        return (HelperEnum[]) RawHelper.class.getDeclaredMethod("array").invoke(null);
    }
    public static void main(String[] args) throws Exception {
        if(!RawEnum.class.isEnum() || !java.lang.reflect.Modifier.isPublic(
            RawEnum.class.getDeclaredConstructor(String.class,int.class).getModifiers()))
            throw new AssertionError("original JVM enum access was silently normalized");
        if(!java.lang.reflect.Modifier.isPrivate(
            HelperEnum.class.getDeclaredConstructor(String.class,int.class).getModifiers()))
            throw new AssertionError("Java enum constructor must remain private");
        HelperEnum[] source = HelperEnum.source(), original = raw();
        if(!Arrays.equals(source, original) || source == original) throw new AssertionError();
        for(int i=0;i<source.length;i++) if(source[i].ordinal()!=i) throw new AssertionError();
        source[0] = HelperEnum.ERROR; original[1] = HelperEnum.ERROR;
        if(HelperEnum.source()[0] != HelperEnum.STARTED ||
           raw()[1] != HelperEnum.PROGRESS) throw new AssertionError();
        if(HelperEnum.source() == HelperEnum.source() ||
           raw() == raw()) throw new AssertionError();
    }
}
"#,
    )
    .unwrap();
    for (tool, args) in [
        (
            "javac",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "HelperEnum.java",
                "MakeOracle.java",
            ],
        ),
        (
            "java",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "-cp",
                ".",
                "MakeOracle",
            ],
        ),
        ("javac", vec!["Check.java"]),
        ("java", vec!["-cp", ".", "Check"]),
    ] {
        let result = Command::new(tool)
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::write(
        root.join("BadEnum.java"),
        "public enum BadEnum { VALUE; public BadEnum() {} }",
    )
    .unwrap();
    let invalid = Command::new("javac")
        .arg("BadEnum.java")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("modifier public not allowed here"));
    std::fs::remove_dir_all(root).unwrap();
}
