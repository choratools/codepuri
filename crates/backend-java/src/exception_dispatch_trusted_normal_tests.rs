//! A genuinely Exception-typed initial is not an untrusted Object capture.
#![allow(clippy::unwrap_used)]
use crate::SourceResult;
pub(super) fn assert_source(result: SourceResult) {
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    let dir = std::env::temp_dir().join(format!(
        "trusted-normal-{}-{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("test")
            .replace("::", "_")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class HandlerPhiHarness {{
static String log=""; static Exception raised; static Exception seen;
static int raise(Exception e,int old)throws Exception{{log+="r";if(raised!=null)throw raised;return old;}}
static void observe(Exception e,int old){{log+="o";seen=e;if(old!=7)throw new AssertionError("snapshot");}}
static String original(Exception e,boolean first,int old){{
 Exception selected;
 if(first){{try{{raise(e,old);return "normal";}}catch(Exception caught){{selected=caught;}}}}
 else{{try{{raise(e,old);return "normal";}}catch(Exception caught){{selected=e;}}}}
 observe(selected,old);return selected.toString();
}}
{source}
public static void main(String[]args){{int n=0;Exception input=new Exception("input");
for(Exception failure:new Exception[]{{null,new java.io.IOException("caught"),new RuntimeException("rt")}})
for(boolean first:new boolean[]{{false,true}}){{
 raised=failure;seen=null;log="";String before=original(input,first,7);Exception identity=seen;String effects=log;
 seen=null;log="";String after=generated(input,first,7);
 if(!before.equals(after)||seen!=identity||!log.equals(effects))throw new AssertionError("selected identity/order");n++;
}}System.out.print("trusted-normal:"+n);}}
}}"#
    );
    std::fs::write(dir.join("HandlerPhiHarness.java"), java).unwrap();
    for home in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let compiled = std::process::Command::new(format!("{home}/bin/javac"))
            .current_dir(&dir)
            .args(["--release", "17", "HandlerPhiHarness.java"])
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let run = std::process::Command::new(format!("{home}/bin/java"))
            .current_dir(&dir)
            .args(["-Xverify:all", "-cp", ".", "HandlerPhiHarness"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout), "trusted-normal:6");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
