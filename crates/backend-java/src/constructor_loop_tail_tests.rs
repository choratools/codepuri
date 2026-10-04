#![allow(clippy::unwrap_used)]
use super::*;
fn label(b: BlockId) -> String {
    format!("{b:?}")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

struct Fixture {
    cfg: Cfg<&'static str>,
    header: BlockId,
    suffix: HashSet<BlockId>,
}
fn fixture() -> Fixture {
    let mut cfg = Cfg::new("super");
    let header = cfg.add_block("header");
    let d = cfg.add_block("isD");
    let c = cfg.add_block("isC");
    let b = cfg.add_block("isB");
    let nd = cfg.add_block("newD");
    let addd = cfg.add_block("addD");
    let nc = cfg.add_block("newC");
    let nb = cfg.add_block("newB");
    let bad = cfg.add_block("throw");
    let phi = cfg.add_block("phi");
    let add = cfg.add_block("add");
    let name = cfg.add_block("name");
    let named = cfg.add_block("hasName");
    let put = cfg.add_block("put");
    let inc = cfg.add_block("inc");
    let ret = cfg.add_block("return");
    for (a, z, k) in [
        (cfg.entry(), header, EdgeKind::Unconditional),
        (header, d, EdgeKind::BranchTrue),
        (header, ret, EdgeKind::BranchFalse),
        (d, nd, EdgeKind::BranchTrue),
        (d, c, EdgeKind::BranchFalse),
        (nd, addd, EdgeKind::Unconditional),
        (addd, inc, EdgeKind::Unconditional),
        (c, nc, EdgeKind::BranchTrue),
        (c, b, EdgeKind::BranchFalse),
        (b, nb, EdgeKind::BranchTrue),
        (b, bad, EdgeKind::BranchFalse),
        (nc, phi, EdgeKind::Unconditional),
        (nb, phi, EdgeKind::Unconditional),
        (phi, add, EdgeKind::Unconditional),
        (add, name, EdgeKind::Unconditional),
        (name, named, EdgeKind::Unconditional),
        (named, put, EdgeKind::BranchTrue),
        (named, inc, EdgeKind::BranchFalse),
        (put, inc, EdgeKind::Unconditional),
        (inc, header, EdgeKind::Unconditional),
    ] {
        cfg.add_edge(a, z, k).unwrap();
    }
    let suffix = cfg.block_ids().filter(|b| *b != cfg.entry()).collect();
    Fixture {
        cfg,
        header,
        suffix,
    }
}
fn trace(
    f: &Fixture,
    p: Option<&Placements>,
    children: &[u8],
    fail: Option<&str>,
) -> (Vec<String>, Option<String>) {
    let mut at = p.map_or(f.cfg.entry(), |p| p.graph.entry());
    let mut index = 0;
    let mut incoming = None;
    let mut out = vec![];
    for _ in 0..1024 {
        let original = p.map_or(at, |p| p.blocks[&at]);
        let event = *f.cfg.block(original).unwrap().payload();
        out.push(format!("{event}:{index}"));
        if fail == Some(event) || event == "throw" {
            return (out, Some(event.into()));
        }
        if event == "return" {
            return (out, None);
        }
        if event == "phi" {
            let edge = f.cfg.edge(incoming.unwrap()).unwrap();
            out.push(format!(
                "selected:{}",
                f.cfg.block(edge.source()).unwrap().payload()
            ));
        }
        let selected = match event {
            "header" => Some(index < children.len()),
            "isD" => Some(children[index] == 0),
            "isC" => Some(children[index] == 1),
            "isB" => Some(children[index] == 2),
            "hasName" => Some(index % 2 == 0),
            "inc" => {
                index += 1;
                None
            }
            _ => None,
        };
        let candidates = if let Some(p) = p {
            p.graph.block(at).unwrap().outgoing()
        } else {
            f.cfg.block(at).unwrap().outgoing()
        };
        let e = *candidates
            .iter()
            .find(|e| {
                let kind = if let Some(p) = p {
                    p.graph.edge(**e).unwrap().kind()
                } else {
                    f.cfg.edge(**e).unwrap().kind()
                };
                match selected {
                    None => *kind == EdgeKind::Unconditional,
                    Some(true) => *kind == EdgeKind::BranchTrue,
                    Some(false) => *kind == EdgeKind::BranchFalse,
                }
            })
            .unwrap();
        incoming = Some(p.map_or(e, |p| p.edges[&e]));
        at = if let Some(p) = p {
            p.graph.edge(e).unwrap().target()
        } else {
            f.cfg.edge(e).unwrap().target()
        };
    }
    panic!("bounded fixture did not terminate");
}
#[test]
fn placements_preserve_selected_phi_edges_loop_tests_effect_order_and_exception_prefixes() {
    let f = fixture();
    let p = expand(&f.cfg, &f.suffix).unwrap();
    p.graph.validate().unwrap();
    assert!(p.graph.block_count() > f.cfg.block_count());
    assert_eq!(p.blocks.values().filter(|b| **b == f.header).count(), 1);
    let a = structure(&f.cfg, &f.suffix).unwrap();
    let b = structure(&f.cfg, &f.suffix).unwrap();
    assert_eq!(a, b);
    for children in [
        vec![],
        vec![0],
        vec![1],
        vec![2],
        vec![0, 1, 2, 1, 0],
        vec![1, 3],
        vec![2, 2, 2],
    ] {
        for fail in [
            None,
            Some("newD"),
            Some("newC"),
            Some("newB"),
            Some("add"),
            Some("name"),
            Some("put"),
        ] {
            assert_eq!(
                trace(&f, None, &children, fail),
                trace(&f, Some(&p), &children, fail)
            );
        }
    }
    assert_eq!(a.edges.len(), f.cfg.edge_ids().count());
}
#[test]
fn handlers_irreducible_entries_and_non_suffix_loops_are_not_placed() {
    let mut f = fixture();
    assert!(expand(&f.cfg, &HashSet::new()).is_err());
    let extra = f.cfg.add_block("handler");
    let source = f
        .cfg
        .block_ids()
        .find(|b| *f.cfg.block(*b).unwrap().payload() == "newC")
        .unwrap();
    f.cfg
        .add_edge(source, extra, EdgeKind::Exception("catch".into()))
        .unwrap();
    assert!(expand(&f.cfg, &f.suffix).is_err());
    let mut f = fixture();
    let inside = f
        .cfg
        .block_ids()
        .find(|b| *f.cfg.block(*b).unwrap().payload() == "phi")
        .unwrap();
    f.cfg
        .add_edge(f.cfg.entry(), inside, EdgeKind::Unconditional)
        .unwrap();
    assert!(expand(&f.cfg, &f.suffix).is_err());
}

#[test]
#[ignore = "requires JDK25 ClassFile API and JDK17 runtime"]
fn loop_tail_placements_match_original_jvm_edges_allocations_and_exception_order() {
    let f = fixture();
    let plan = structure(&f.cfg, &f.suffix).unwrap();
    fn emit(r: &ControlRegion, f: &Fixture, out: &mut String) {
        let action = |b: BlockId| {
            let name = f.cfg.block(b).unwrap().payload();
            let mut s = format!("effect(\"{name}\",i);\n");
            match *name {
                "inc" => s.push_str("i++;\n"),
                "return" => s.push_str("return;\n"),
                "throw" => s.push_str("throw BAD;\n"),
                _ => {}
            }
            s
        };
        match r {
            ControlRegion::Block(b) => out.push_str(&action(*b)),
            ControlRegion::Sequence(items) => {
                for item in items {
                    emit(item, f, out)
                }
            }
            ControlRegion::If {
                header,
                then_region,
                else_region,
                ..
            } => {
                out.push_str(&action(*header));
                out.push_str(&format!(
                    "if(test(\"{}\",i,children)){{\n",
                    f.cfg.block(*header).unwrap().payload()
                ));
                emit(then_region, f, out);
                out.push_str("}else{\n");
                emit(else_region, f, out);
                out.push_str("}\n");
            }
            ControlRegion::Loop { header, body, .. } => {
                out.push_str(&format!("loop{}:while(true){{\n", label(*header)));
                emit(body, f, out);
                out.push_str("}\n");
            }
            ControlRegion::Transfer { kind, .. } => match kind {
                TransferKind::Break { header } => {
                    out.push_str(&format!("break loop{};\n", label(*header)))
                }
                TransferKind::Continue { header } => {
                    out.push_str(&format!("continue loop{};\n", label(*header)))
                }
            },
        }
    }
    let mut source = String::new();
    emit(&plan.root, &f, &mut source);
    let dir = std::env::temp_dir().join(format!("constructor-loop-tail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"import java.util.*;public class LoopTail {{
static String events="";static Object selected;static String name;static List<Object> list=new ArrayList<>();static Map<String,Object> map=new HashMap<>();static String fail="";static final RuntimeException PRIMARY=new RuntimeException("primary"),BAD=new IllegalStateException("bad");static final Error ERROR=new Error("error");
static class D{{}}static class C{{}}static class B{{}}
static void effect(String event,int i){{events+=event+":"+i+";";if(fail.equals(event))throw PRIMARY;if(fail.equals("!"+event))throw ERROR;switch(event){{case "newD":selected=new D();break;case "newC":selected=new C();break;case "newB":selected=new B();break;case "addD":case "add":list.add(selected);break;case "name":name="n"+i;break;case "put":map.put(name,selected);break;}}}}
static boolean test(String e,int i,int[]children){{return switch(e){{case "header"->i<children.length;case "isD"->children[i]==0;case "isC"->children[i]==1;case "isB"->children[i]==2;case "hasName"->i%2==0;default->throw new AssertionError(e);}};}}
static void original(int[]children){{}}
static void generated(int[]children){{int i=0;{source}}}
static String run(boolean lowered,int[]children,String failure){{events="";selected=null;name=null;list.clear();map.clear();fail=failure;Throwable escaped=null;try{{if(lowered)generated(children);else original(children);}}catch(Throwable t){{escaped=t;}}return events+":"+list.size()+":"+map.keySet()+":"+(escaped==PRIMARY)+":"+(escaped==ERROR)+":"+(escaped==BAD);}}
public static void main(String[]args){{int[][]inputs={{{{}},{{0}},{{1}},{{2}},{{0,1,2,1,0}},{{1,3}},{{2,2,2}}}};String[]failures={{"","newD","newC","newB","add","name","put","!newC","!add","!put"}};int n=0;for(int[]input:inputs)for(String failure:failures){{String before=run(false,input,failure),after=run(true,input,failure);if(!before.equals(after))throw new AssertionError(before+" != "+after);n++;}}System.out.println("cases="+n);}}
}}"#
    );
    std::fs::write(dir.join("LoopTail.java"), java).unwrap();
    let mut bytecode = String::new();
    for b in f.cfg.block_ids() {
        bytecode.push_str(&format!("Label b{}=c.newLabel();", label(b)));
    }
    bytecode.push_str("c.iconst_0().istore(1);");
    for b in f.cfg.block_ids() {
        let name = f.cfg.block(b).unwrap().payload();
        bytecode.push_str(&format!("c.labelBinding(b{}).ldc(\"{name}\").iload(1).invokestatic(self,\"effect\",MethodTypeDesc.ofDescriptor(\"(Ljava/lang/String;I)V\"));",label(b)));
        match *name {
            "return" => bytecode.push_str("c.return_();"),
            "throw" => bytecode.push_str(
                "c.getstatic(self,\"BAD\",ClassDesc.of(\"java.lang.RuntimeException\")).athrow();",
            ),
            _ => {
                if *name == "inc" {
                    bytecode.push_str("c.iinc(1,1);");
                }
                let edges = f.cfg.block(b).unwrap().outgoing();
                if edges.len() == 1 {
                    bytecode.push_str(&format!(
                        "c.goto_(b{});",
                        label(f.cfg.edge(edges[0]).unwrap().target())
                    ));
                } else {
                    let yes = edges
                        .iter()
                        .find(|e| f.cfg.edge(**e).unwrap().kind() == &EdgeKind::BranchTrue)
                        .unwrap();
                    let no = edges
                        .iter()
                        .find(|e| f.cfg.edge(**e).unwrap().kind() == &EdgeKind::BranchFalse)
                        .unwrap();
                    bytecode.push_str(&format!("c.ldc(\"{name}\").iload(1).aload(0).invokestatic(self,\"test\",MethodTypeDesc.ofDescriptor(\"(Ljava/lang/String;I[I)Z\")).ifeq(b{}).goto_(b{});",label(f.cfg.edge(*no).unwrap().target()),label(f.cfg.edge(*yes).unwrap().target())));
                }
            }
        }
    }
    let transform = format!(
        r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;public class Original{{public static void main(String[]args)throws Exception{{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("LoopTail");Path path=Path.of("LoopTail.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,e)->{{if(e instanceof MethodModel mm&&mm.methodName().equalsString("original"))cb.transformMethod(mm,(mb,me)->{{if(me instanceof CodeModel)mb.withCode(c->{{{bytecode}}});else mb.with(me);}});else cb.with(e);}}));}}}}"#
    );
    std::fs::write(dir.join("Original.java"), transform).unwrap();
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |home: &str, tool: &str, args: &[&str]| {
        let result = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };
    run(&home, "javac", &["--release", "17", "LoopTail.java"]);
    run(&home, "javac", &["Original.java"]);
    run(&home, "java", &["Original"]);
    assert_eq!(run(&home, "java", &["LoopTail"]).trim(), "cases=70");
    assert_eq!(
        run("/home/jtjisgod/tools/jdk-17.0.13+11", "java", &["LoopTail"]).trim(),
        "cases=70"
    );
    eprintln!("original JVM loop-tail oracle: {}", dir.display());
}
