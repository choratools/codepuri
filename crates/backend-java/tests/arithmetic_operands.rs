//! Int promotion must retain signed, unsigned and shift-distance semantics.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../src/arithmetic_operands.rs"]
mod arithmetic_operands;
pub use backend_java::{JavaType, Unsupported};
use managed_ir::ops::ArithOp;

#[test]
fn only_exact_width_or_int_promotion_is_accepted() {
    use JavaType::*;
    for left in [Byte, Short, Char, Int] {
        for right in [Byte, Short, Char, Int] {
            for op in [
                ArithOp::Add,
                ArithOp::And,
                ArithOp::ShiftLeft,
                ArithOp::UnsignedShiftRight,
            ] {
                arithmetic_operands::validate(op, &Int, &left, &right).unwrap();
            }
        }
        arithmetic_operands::validate(ArithOp::ShiftRight, &Long, &Long, &left).unwrap();
    }
    for mixed in [
        Long,
        Float,
        Double,
        Boolean,
        Class("java.lang.Object".into()),
    ] {
        assert!(arithmetic_operands::validate(ArithOp::Add, &Int, &Int, &mixed).is_err());
        assert!(arithmetic_operands::validate(ArithOp::And, &Int, &mixed, &Int).is_err());
    }
    for wide in [Long, Float, Double] {
        assert!(arithmetic_operands::validate(ArithOp::Add, &wide, &wide, &Int).is_err());
    }
    for narrow in [Byte, Short, Char] {
        assert!(arithmetic_operands::validate(ArithOp::Add, &narrow, &narrow, &narrow).is_err());
    }
    assert!(arithmetic_operands::validate(ArithOp::Add, &Boolean, &Boolean, &Boolean).is_err());
    arithmetic_operands::validate(ArithOp::Or, &Boolean, &Boolean, &Boolean).unwrap();
    assert!(arithmetic_operands::validate(ArithOp::And, &Float, &Float, &Float).is_err());
    assert!(arithmetic_operands::validate(ArithOp::ShiftLeft, &Double, &Double, &Int).is_err());
}

#[test]
fn javac_executes_narrow_masks_and_shift_boundaries() {
    arithmetic_operands::validate(
        ArithOp::And,
        &JavaType::Int,
        &JavaType::Byte,
        &JavaType::Int,
    )
    .unwrap();
    arithmetic_operands::validate(
        ArithOp::UnsignedShiftRight,
        &JavaType::Int,
        &JavaType::Short,
        &JavaType::Char,
    )
    .unwrap();
    arithmetic_operands::validate(
        ArithOp::ShiftLeft,
        &JavaType::Int,
        &JavaType::Char,
        &JavaType::Byte,
    )
    .unwrap();
    let root =
        std::env::temp_dir().join(format!("decomphify-int-promotion-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = r#"public class IntPromotion {
      static short mask(byte b) { int v=b & 255; return (short)v; }
      static int shifted(short s,char n) { return s >>> n; }
      static int charShift(char c,byte n) { return c << n; }
      public static void main(String[] args) {
        int[] bytes={-128,-1,0,1,127};
        for(int b:bytes) if(mask((byte)b)!=(b&255)) throw new AssertionError("mask");
        int[] shorts={-32768,-1,0,1,32767};
        int[] chars={0,1,31,32,33,65535};
        for(int s:shorts) for(int n:chars)
          if(shifted((short)s,(char)n)!=((int)(short)s >>> (n&31))) throw new AssertionError("short shift");
        for(int c:chars) for(int n:bytes)
          if(charShift((char)c,(byte)n)!=((int)(char)c << (n&31))) throw new AssertionError("char shift");
      }
    }"#;
    let file = root.join("IntPromotion.java");
    std::fs::write(&file, source).unwrap();
    let javac = std::process::Command::new("javac")
        .arg(&file)
        .output()
        .expect("JDK required for arithmetic semantic regression");
    assert!(
        javac.status.success(),
        "{}",
        String::from_utf8_lossy(&javac.stderr)
    );
    let java = std::process::Command::new("java")
        .arg("-cp")
        .arg(&root)
        .arg("IntPromotion")
        .output()
        .unwrap();
    assert!(
        java.status.success(),
        "{}",
        String::from_utf8_lossy(&java.stderr)
    );
}
