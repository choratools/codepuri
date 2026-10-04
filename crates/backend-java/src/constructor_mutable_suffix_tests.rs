// Original Samsung constructors #10372 (no finals) and #29753 (mutable b in
// loop, final f afterwards) must not be rejected by final-only suffix guards.
struct MutableFieldResolver {
    flags: Option<u32>,
    wrong_type: bool,
}

impl JavaResolver for MutableFieldResolver {
    fn class(&self, c: ClassRef) -> Option<String> {
        R.class(c)
    }
    fn string(&self, s: StringRef) -> Option<Vec<u16>> {
        R.string(s)
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        R.method(m)
    }
    fn field_access(&self, f: FieldRef) -> Option<u32> {
        if f.index() == 0 {
            Some(0x11)
        } else {
            self.flags
        }
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: if f.index() == 0 { "value" } else { "mutable" }.into(),
            ty: if self.wrong_type && f.index() == 1 {
                JavaType::Class("java.lang.String".into())
            } else {
                JavaType::Int
            },
            is_static: false,
        })
    }
}

fn mutable_loop_fixture() -> Fixture {
    fixture(5)
}

#[test]
fn mutable_loop_preserves_post_loop_final_assignment_and_runtime() {
    let f = mutable_loop_fixture();
    let resolver = MutableFieldResolver {
        flags: Some(1),
        wrong_type: false,
    };
    let mut e = emitter(&f, &resolver);
    let mut seen = HashSet::from([f.ir.cfg().entry()]);
    emit_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen).unwrap();
    let suffix = e.lines.join("\n");
    assert!(suffix.contains("this.mutable ="));
    assert_eq!(suffix.matches("this.value =").count(), 1);
    assert!(suffix.find("this.mutable =").unwrap() < suffix.find("this.value =").unwrap());
    let dir = std::env::temp_dir().join(format!(
        "decomphify-mutable-final-loop-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "public class Fixture {{ final int value; int mutable; Fixture(int p0,int p1) {{ super(); {suffix} }} public static void main(String[] a) {{ for(int s=-3;s<=3;s++) for(int n=-3;n<=6;n++) {{ Fixture f=new Fixture(s,n); if(f.value!=Math.max(s,n) || f.mutable!=(s<n?n-1:0)) throw new AssertionError(); }} }} }}"
    );
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let javac = std::process::Command::new("javac")
        .arg(dir.join("Fixture.java"))
        .output()
        .unwrap();
    assert!(
        javac.status.success(),
        "{}",
        String::from_utf8_lossy(&javac.stderr)
    );
    let run = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("Fixture")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn mutable_loop_requires_flags_and_preserves_final_and_store_type_rejections() {
    for (flags, wrong_type) in [(None, false), (Some(0x11), false), (Some(1), true)] {
        let f = mutable_loop_fixture();
        let resolver = MutableFieldResolver { flags, wrong_type };
        let mut e = emitter(&f, &resolver);
        let mut seen = HashSet::from([f.ir.cfg().entry()]);
        assert!(
            emit_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen)
                .is_err()
        );
    }
}

#[test]
fn true_final_store_requires_original_this_receiver() {
    let f = mutable_loop_fixture();
    let resolver = MutableFieldResolver {
        flags: Some(1),
        wrong_type: false,
    };
    let mut e = emitter(&f, &resolver);
    e.values
        .insert(f.this, ("other".into(), JavaType::Class("Fixture".into())));
    let mut seen = HashSet::from([f.ir.cfg().entry()]);
    assert!(
        emit_initialized_suffix(&mut e, &method(), f.entry, Some(f.incoming), &mut seen).is_err()
    );
}
