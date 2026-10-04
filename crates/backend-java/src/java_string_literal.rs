//! Native Java string constants preserving the original UTF-16 code units.
use crate::Unsupported;

/// Java 17 source literal backed by a CONSTANT_String, without runtime allocation.
/// Unicode escapes are translated before lexing (JLS 3.3); raw backslashes must
/// therefore use octal escapes, never Unicode escapes. Three octal digits also
/// prevent an adjacent digit from extending the escape.
pub(crate) fn render(units: &[u16]) -> Result<String, Unsupported> {
    let bytes = units.iter().try_fold(0usize, |bytes, unit| {
        let width = match unit {
            0 => 2,
            1..=0x7f => 1,
            0x80..=0x7ff => 2,
            _ => 3,
        };
        (bytes + width <= 65535).then_some(bytes + width)
    });
    if bytes.is_none() {
        return Err(Unsupported::Instruction(
            "string exceeds CONSTANT_Utf8 modified UTF-8 byte limit".into(),
        ));
    }
    // javac 17 rejects 65535 UTF-16 units even when their encoded byte length
    // fits the classfile limit. Constant concatenation does not avoid this limit.
    if units.len() >= 65535 {
        return Err(Unsupported::Instruction(
            "string exceeds Java 17 compiler literal unit limit".into(),
        ));
    }
    let mut source = String::from("\"");
    for unit in units {
        match *unit {
            10 => source.push_str("\\n"),
            13 => source.push_str("\\r"),
            0..=31 | 34 | 92 | 127 => source.push_str(&format!("\\{unit:03o}")),
            32..=126 => source.push(char::from(*unit as u8)),
            _ => source.push_str(&format!("\\u{unit:04x}")),
        }
    }
    source.push('"');
    Ok(source)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::render;
    use std::{fmt::Write, process::Command};
    const JDK: &str = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";

    #[test]
    fn java_string_literal_budget_is_modified_utf8_and_compiler_bounded() {
        assert!(render(&vec![0x800; 21845]).is_ok());
        assert!(render(&vec![0x800; 21846]).is_err());
        assert!(render(&vec![0; 32767]).is_ok());
        assert!(render(&vec![0; 32768]).is_err());
        assert!(render(&vec![65; 65534]).is_ok());
        assert!(render(&vec![65; 65535]).is_err());
    }

    #[test]
    fn java_string_literal_jdk_all_utf16_identity_and_ldc() {
        let dir = std::env::temp_dir().join(format!("java-string-literal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut source = String::from("public class LiteralProof {\n");
        for chunk in 0..64u32 {
            let units: Vec<u16> = (chunk * 1024..(chunk + 1) * 1024)
                .map(|v| v as u16)
                .collect();
            let literal = render(&units).unwrap();
            assert!(literal.is_ascii());
            writeln!(source,"static String s{chunk}() {{return {literal};}} static String d{chunk}() {{return {literal};}}").unwrap();
        }
        let adversarial: Vec<u16> = "\\u000a\\\\u005c\r\n\0\"01234567\\"
            .encode_utf16()
            .chain([0xd800, 0xdc00, 0xdfff, 0xffff])
            .collect();
        writeln!(
            source,
            "static String tricky() {{return {};}}",
            render(&adversarial).unwrap()
        )
        .unwrap();
        writeln!(
            source,
            "static String boundary() {{return {};}}",
            render(&vec![0x800; 21845]).unwrap()
        )
        .unwrap();
        source.push_str("public static void main(String[] args) { String s;\n");
        for chunk in 0..64u32 {
            writeln!(source,"s=s{chunk}(); if(s != d{chunk}() || s != s.intern() || s.length()!=1024) throw new AssertionError(); for(int i=0;i<1024;i++) if(s.charAt(i)!=(char)({chunk}*1024+i)) throw new AssertionError();").unwrap();
        }
        let expected = adversarial
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(source,"int[] expected={{{expected}}}; s=tricky(); if(s.length()!=expected.length) throw new AssertionError(); for(int i=0;i<expected.length;i++) if(s.charAt(i)!=expected[i]) throw new AssertionError(); if(boundary().length()!=21845) throw new AssertionError(); }} }}").unwrap();
        std::fs::write(dir.join("LiteralProof.java"), source).unwrap();
        for tool in ["javac", "java"] {
            let mut command = Command::new(format!("{JDK}/{tool}"));
            command.current_dir(&dir);
            if tool == "javac" {
                command.args(["--release", "17", "LiteralProof.java"]);
            } else {
                command.args(["-cp", ".", "LiteralProof"]);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let output = Command::new(format!("{JDK}/javap"))
            .current_dir(&dir)
            .args(["-c", "-p", "LiteralProof"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let disassembly = String::from_utf8(output.stdout).unwrap();
        std::fs::write(dir.join("javap.txt"), &disassembly).unwrap();
        // Only inspect constant-return methods: the test driver intentionally
        // allocates its expected array and invokes intern as an identity oracle.
        let methods = disassembly
            .split("  public static void main")
            .next()
            .unwrap();
        assert!(
            !methods
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some("new"))
                && !methods.contains("newarray")
                && !methods.contains("// class java/lang/String")
                && !methods.contains("String.intern")
                && !methods.contains("invokedynamic")
        );
        assert_eq!(methods.matches("// String").count(), 130);
        for chunk in 0..64 {
            let load = |name: String| {
                let body = methods
                    .split(&format!("static java.lang.String {name}();"))
                    .nth(1)
                    .unwrap()
                    .split("areturn")
                    .next()
                    .unwrap();
                body.lines()
                    .find(|line| line.contains("ldc"))
                    .unwrap()
                    .split("//")
                    .next()
                    .unwrap()
                    .split('#')
                    .nth(1)
                    .unwrap()
                    .trim()
                    .to_owned()
            };
            assert_eq!(load(format!("s{chunk}")), load(format!("d{chunk}")));
        }
    }
}
