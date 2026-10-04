//! Raw opcode loops at original entry retain invocation values and effects.
#![allow(clippy::unwrap_used)]
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType, SourceResult};
use dex_decompiler::{DexPasses, TypeContext};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
};
fn code(units: &[u16], registers: u16, incoming: u16) -> CodeItem {
    CodeItem {
        registers_size: registers,
        ins_size: incoming,
        outs_size: 2,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|w| w.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
struct FixtureTypes;
impl TypeContext for FixtureTypes {
    fn initial_register_type(&self, r: u16, wide: bool) -> Option<ManagedType> {
        if wide {
            return None;
        }
        match r {
            1 => self.type_at(2),
            2 => self.type_at(0),
            _ => None,
        }
    }
    fn type_at(&self, i: u32) -> Option<ManagedType> {
        (i < 4).then_some(ManagedType::Reference(ClassRef::new(i)))
    }
    fn method_return(&self, m: u32) -> Option<ManagedType> {
        self.type_at(if m == 0 { 1 } else { 3 })
    }
    fn method_params(&self, m: u32, p: &mut Vec<ManagedType>) -> bool {
        p.clear();
        if m == 1 {
            p.push(self.type_at(0).unwrap());
        }
        m < 2
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        self.type_at(3)
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        match r.index() {
            0 => Some("Fixture.View".into()),
            1 => Some("Fixture.Parent".into()),
            2 => Some("Fixture.Group".into()),
            3 => Some("Fixture.Info".into()),
            _ => None,
        }
    }
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        self.class(ClassRef::new(r.index())).map(JavaType::Class)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (owner, name, parameters, ret) = match r.index() {
            0 => (0, "getParent", vec![], 1),
            1 => (
                2,
                "infoForChild",
                vec![JavaType::Class(self.class(ClassRef::new(0))?)],
                3,
            ),
            _ => return None,
        };
        Some(JavaMethod {
            owner: self.class(ClassRef::new(owner))?,
            name: name.into(),
            is_static: false,
            return_type: JavaType::Class(self.class(ClassRef::new(ret))?),
            parameters,
            throws: vec![],
        })
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        (r.index() < 2).then_some(1)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || *expected == JavaType::Class("java.lang.Object".into())
                && matches!(actual, JavaType::Class(_) | JavaType::Array(_))
            || *actual == JavaType::Class("Fixture.Group".into())
                && matches!(expected,JavaType::Class(n) if n=="Fixture.View" || n=="Fixture.Parent")
    }
}
fn compile_run(name: &str, text: &str) {
    let dir =
        std::env::temp_dir().join(format!("decomp-merge-entry-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), text).unwrap();
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let compiled = std::process::Command::new(home.join("bin/javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = std::process::Command::new(home.join("bin/java"))
        .args(["-cp", ".", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn original_entry_goto_keeps_backend_acceptance_and_drops_only_other_seams() {
    for (units, expected) in [
        (vec![0x0228, 0x7012, 0x1012, 0x000f], 1),
        (vec![0x1012, 0x0128, 0x2112, 0x010f], 2),
    ] {
        let ssa = DexPasses::run_merged(&code(&units, 2, 0)).unwrap();
        let signature = JavaMethod {
            owner: "Fixture".into(),
            name: "value".into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![],
            throws: vec![],
        };
        let result =
            backend_java::emit_method(ssa.ir(), &signature, &Default::default(), &Resolver);
        let SourceResult::Complete { source, .. } = result else {
            panic!("entry goto regression: {result:?}")
        };
        compile_run(
            &format!("goto-{expected}"),
            &format!(
                "public class Fixture {{ {source} public static void main(String[] args) {{if(value()!={expected})throw new AssertionError();}} }}"
            ),
        );
    }
}
#[test]
fn retained_entry_goto_rejects_instruction_cfg_disagreement() {
    use managed_ir::{
        insn::ManagedInsn,
        method::{MethodIr, SsaForm},
    };
    let ssa = DexPasses::run_merged(&code(&[0x0228, 0x7012, 0x1012, 0x000f], 2, 0)).unwrap();
    let ir = ssa.ir();
    let branch_id = *ir.cfg().block(ir.cfg().entry()).unwrap().payload();
    let ManagedInsn::Branch {
        condition: None,
        then_edge,
        else_edge: None,
    } = ir.instruction(branch_id).unwrap()
    else {
        panic!("original entry goto was not retained")
    };
    // Use another real edge, preserving the valid CFG and changing only the
    // instruction's encoded transfer. It must not become a generic no-op.
    let wrong_edge = ir.cfg().edge_ids().find(|edge| edge != then_edge).unwrap();
    let mut cfg = decomp_core::cfg::Cfg::new(branch_id);
    let mut blocks = std::collections::HashMap::from([(ir.cfg().entry(), cfg.entry())]);
    for block in ir
        .cfg()
        .block_ids()
        .filter(|block| *block != ir.cfg().entry())
    {
        blocks.insert(
            block,
            cfg.add_block(*ir.cfg().block(block).unwrap().payload()),
        );
    }
    let mut edges = std::collections::HashMap::new();
    for edge_id in ir.cfg().edge_ids() {
        let edge = ir.cfg().edge(edge_id).unwrap();
        edges.insert(
            edge_id,
            cfg.add_edge(
                blocks[&edge.source()],
                blocks[&edge.target()],
                edge.kind().clone(),
            )
            .unwrap(),
        );
    }
    cfg.validate().unwrap();
    let mut instructions = ir.instructions().clone();
    instructions[branch_id] = ManagedInsn::Branch {
        condition: None,
        then_edge: edges[&wrong_edge],
        else_edge: None,
    };
    let malformed = MethodIr::<SsaForm>::from_parts_with_registers(
        cfg,
        instructions,
        ir.values().clone(),
        ir.register_slots().clone(),
    );
    let signature = JavaMethod {
        owner: "Fixture".into(),
        name: "value".into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![],
        throws: vec![],
    };
    let result = backend_java::emit_method(&malformed, &signature, &Default::default(), &Resolver);
    assert!(
        matches!(
            result,
            SourceResult::Unsupported(backend_java::Unsupported::ControlFlow(ref reason))
                if reason == "unconditional instruction/CFG edge mismatch"
        ),
        "{result:?}"
    );
}
#[test]
fn actual_raw_entry_loop_emitted_method_matches_original_operation_trace() {
    // Original ViewPager.infoForAnyChild opcode shape, exact physical operands.
    let units = [
        0x106e, 0, 2, 0x000c, 0x1032, 12, 0x0220, 0, 0x0239, 4, 0x0112, 0x0111, 0x0207, 0x021f, 0,
        0xf128, 0x206e, 1, 0x0021, 0x010c, 0x0111,
    ];
    let item = code(&units, 3, 2);
    let ssa = DexPasses::run_ssa_with(&item, &FixtureTypes).unwrap();
    let signature = JavaMethod {
        owner: "Fixture.Group".into(),
        name: "infoForAnyChild".into(),
        is_static: false,
        return_type: JavaType::Class("Fixture.Info".into()),
        parameters: vec![JavaType::Class("Fixture.View".into())],
        throws: vec![],
    };
    let parameters = decomp::dex_java::parameter_bindings(ssa.ir(), &signature, 3, 2).unwrap();
    let result = backend_java::emit_method(ssa.ir(), &signature, &parameters, &Resolver);
    let SourceResult::Complete { source, .. } = result else {
        panic!("entry recurrence source: {result:?}")
    };
    let text = format!(
        r#"public class Fixture {{
static String trace="";static final Error sentinel=new Error("identity");
public interface Parent {{}}
public static class View {{ final int id; Parent parent; boolean fail; View(int id) {{this.id=id;}}
public Parent getParent() {{trace+="P"+id+";";if(fail)throw sentinel;return parent;}} }}
public static class ForeignView extends View implements Parent {{ ForeignView(int id) {{super(id);}} }}
public static class Info {{ final int child; Info(int child) {{this.child=child;}} }}
public static class Group extends View implements Parent {{ Group(int id) {{super(id);}}
public Info infoForChild(View child) {{trace+="I"+child.id+";";return new Info(child.id);}}
// Independent register-bytecode oracle for the exact fixture's original opcodes.
public Info reference(View child) {{while(true){{Parent p=child.getParent();if(p==this)return infoForChild(child);if(!(p instanceof View))return null;child=(View)p;}}}}
{source}
}}
public static void main(String[] args) {{
Group root=new Group(1),middle=new Group(2);View leaf=new View(3);leaf.parent=middle;middle.parent=root;
Info result=root.infoForAnyChild(leaf);if(result.child!=2 || !trace.equals("P3;P2;I2;"))throw new AssertionError(trace);
String emittedTrace=trace;trace="";Info oracle=root.reference(leaf);if(oracle.child!=result.child || !trace.equals(emittedTrace))throw new AssertionError(trace);
trace="";View direct=new View(4);direct.parent=root;if(root.infoForAnyChild(direct).child!=4 || !trace.equals("P4;I4;"))throw new AssertionError(trace);
trace="";ForeignView foreign=new ForeignView(6);foreign.parent=root;leaf.parent=foreign;if(root.infoForAnyChild(leaf).child!=6 || !trace.equals("P3;P6;I6;"))throw new AssertionError(trace);leaf.parent=middle;
trace="";View outside=new View(5);outside.parent=new Parent(){{}};if(root.infoForAnyChild(outside)!=null || !trace.equals("P5;"))throw new AssertionError(trace);
trace="";leaf.fail=true;try{{root.infoForAnyChild(leaf);throw new AssertionError();}}catch(Error e){{if(e!=sentinel)throw new AssertionError();}}if(!trace.equals("P3;"))throw new AssertionError(trace);
trace="";leaf.fail=false;middle.fail=true;try{{root.infoForAnyChild(leaf);throw new AssertionError();}}catch(Error e){{if(e!=sentinel)throw new AssertionError();}}if(!trace.equals("P3;P2;"))throw new AssertionError(trace);
trace="";try{{root.infoForAnyChild(null);throw new AssertionError();}}catch(NullPointerException expected){{}}if(!trace.equals(""))throw new AssertionError(trace);
}} }}"#
    );
    compile_run("view-loop", &text);
}

#[test]
#[ignore = "retained Samsung APK and genuine Android/JDK25; one selected ViewPager class production probe"]
fn original_viewpager_21016_production_publication_retains_entry_loop() {
    use decomp::{
        dex_java::{JavaOptions, emit_java},
        output::GeneratedSource,
    };
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &files[0];
    let raw = dex.method_id(21016).unwrap();
    assert_eq!(dex.string(raw.name_idx).unwrap(), "infoForAnyChild");
    let owner = "Landroidx/viewpager/widget/ViewPager;";
    assert_eq!(
        dex.string(
            dex.type_id(u32::from(raw.class_idx))
                .unwrap()
                .descriptor_idx
        )
        .unwrap(),
        owner
    );
    let options = JavaOptions {
        source_release: 25,
        class_filter: Some(owner.into()),
        workers: 1,
        classpath: vec![
            "/tmp/decomphify-source25-java-base-649174/java.base.jar".into(),
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
        ],
        ..JavaOptions::default()
    };
    let mut units = Vec::new();
    let started = std::time::Instant::now();
    let report = emit_java(&files, &options, &mut |unit: &GeneratedSource| {
        units.push(unit.clone());
        Ok(())
    })
    .unwrap();
    let unit = units
        .iter()
        .find(|u| u.provenance.descriptor == owner)
        .unwrap_or_else(|| panic!("no original ViewPager unit: {report:?}"));
    let marker = "    // original method pool index 21016;";
    let start = unit
        .text
        .find(marker)
        .unwrap_or_else(|| panic!("original ViewPager.infoForAnyChild not published: {report:?}"));
    let tail = &unit.text[start..];
    let end = tail[marker.len()..]
        .find("    // original method pool index ")
        .map(|n| n + marker.len())
        .unwrap_or(tail.len());
    let body = &tail[..end];
    assert!(
        body.lines()
            .next()
            .unwrap()
            .contains("complete_body=true; semantic_complete=true; dependency_complete=true"),
        "{body}"
    );
    assert!(body.contains("getParent()"));
    assert!(body.contains("infoForChild("));
    let path = "/tmp/opencode/wave8-original-viewpager-21016.java.txt";
    std::fs::write(path, body).unwrap();
    println!(
        "ORIGINAL PRODUCTION PUBLISHED pool0 method21016 elapsed_s={:.3} bytes={} path={path}",
        started.elapsed().as_secs_f64(),
        body.len()
    );
}
