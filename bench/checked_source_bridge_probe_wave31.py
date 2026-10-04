#!/usr/bin/env python3
"""Standalone checked-source bridge feasibility; no original-method recovery claim."""
import hashlib
import json
import pathlib
import subprocess
import tempfile

SOURCE = r'''
import java.io.IOException;
import java.util.Arrays;

public final class CheckedBridgeProbe {
    static final StringBuilder effects = new StringBuilder();
    static Throwable primary;
    static Throwable cleanupFailure;
    static Throwable argumentFailure;
    static int bridgeEntries;
    static int checks;

    // Compiler-only artifact: not an original input method identity.
    @SuppressWarnings("unchecked")
    private static <T extends Throwable> void rethrow(Throwable failure) throws T {
        bridgeEntries++;
        throw (T) failure;
    }
    @SuppressWarnings("unchecked")
    private static <T extends Throwable> RuntimeException rethrowValue(Throwable failure) throws T {
        bridgeEntries++;
        throw (T) failure;
    }

    static void raise(Throwable failure) throws Exception {
        if (failure instanceof Error) throw (Error) failure;
        if (failure instanceof Exception) throw (Exception) failure;
        if (failure != null) throw new AssertionError("fixture accepts Exception/Error only");
    }
    static int argument() throws Exception { effects.append("argument;"); raise(argumentFailure); return 7; }
    static void invoke(int value) throws Exception {
        if(value != 7) throw new AssertionError();
        effects.append("invoke;"); raise(primary);
    }
    static void cleanup() throws Exception { effects.append("cleanup;"); raise(cleanupFailure); }

    // Catch extent contains exactly the original invocation expression.
    static void bridgedInvoke() {
        try { invoke(argument()); }
        catch(Exception failure) { CheckedBridgeProbe.<RuntimeException>rethrow(failure); }
    }
    static void baselineInvoke() throws Exception { invoke(argument()); }
    static Object valueInvoke(int value) throws Exception {
        if(value!=7) throw new AssertionError();
        effects.append("value;"); raise(primary); return "success";
    }
    static Object baselineValue() throws Exception {
        Object result=valueInvoke(argument()); effects.append("after;"); return result;
    }
    static Object bridgedValue() {
        Object result;
        try { result=valueInvoke(argument()); }
        catch(Exception failure) { throw CheckedBridgeProbe.<RuntimeException>rethrowValue(failure); }
        effects.append("after;"); return result;
    }

    static void baselineFinally() throws Exception {
        try { invoke(argument()); } finally { cleanup(); }
    }
    static void bridgedFinally() {
        try { bridgedInvoke(); }
        finally {
            try { cleanup(); }
            catch(Exception failure) { CheckedBridgeProbe.<RuntimeException>rethrow(failure); }
        }
    }

    // Original captured-primary + suppression structure remains outside bridge.
    static void baselineSuppressed() throws Exception {
        try { invoke(argument()); }
        catch(Throwable original) {
            try { cleanup(); } catch(Throwable second) { original.addSuppressed(second); }
            raise(original); return;
        }
        cleanup();
    }
    static void bridgedSuppressed() {
        try { bridgedInvoke(); }
        catch(Throwable original) {
            try { cleanup(); } catch(Throwable second) { original.addSuppressed(second); }
            CheckedBridgeProbe.<RuntimeException>rethrow(original); return;
        }
        try { cleanup(); }
        catch(Exception failure) { CheckedBridgeProbe.<RuntimeException>rethrow(failure); }
    }
    interface Action { void run() throws Exception; }
    static final class Observation {
        Throwable escaped;
        String effects;
        int entries;
    }
    static Observation observe(Action action) {
        effects.setLength(0); bridgeEntries=0;
        Observation o=new Observation();
        try { action.run(); } catch(Throwable failure) { o.escaped=failure; }
        o.effects=effects.toString(); o.entries=bridgeEntries; return o;
    }
    static void require(boolean condition) { checks++; if(!condition) throw new AssertionError(); }
    static void invocation(Throwable failure, boolean shouldBridge) {
        primary=failure; cleanupFailure=null; argumentFailure=null;
        StackTraceElement[] stack=failure == null ? null : failure.getStackTrace();
        String message=failure == null ? null : failure.getMessage();
        Observation a=observe(CheckedBridgeProbe::baselineInvoke);
        Observation b=observe(CheckedBridgeProbe::bridgedInvoke);
        require(a.escaped==failure && b.escaped==failure);
        require(a.effects.equals("argument;invoke;") && a.effects.equals(b.effects));
        require(b.entries==(shouldBridge ? 1 : 0));
        if(failure!=null) {
            require(Arrays.equals(stack,failure.getStackTrace()));
            require(java.util.Objects.equals(message,failure.getMessage()));
        }
    }
    static void argumentOrdering(Throwable failure, boolean shouldBridge) {
        primary=new IOException("must not invoke"); argumentFailure=failure;
        StackTraceElement[] stack=failure.getStackTrace();
        Observation a=observe(CheckedBridgeProbe::baselineInvoke);
        Observation b=observe(CheckedBridgeProbe::bridgedInvoke);
        require(a.escaped==failure && b.escaped==failure);
        require(a.effects.equals("argument;") && a.effects.equals(b.effects));
        require(Arrays.equals(stack,failure.getStackTrace()));
        require(b.entries==(shouldBridge ? 1 : 0));
        argumentFailure=null;
    }
    static void valueCase(Throwable failure) {
        primary=failure; argumentFailure=null;
        Observation a=observe(() -> { require(baselineValue().equals("success")); });
        Observation b=observe(() -> { require(bridgedValue().equals("success")); });
        require(a.escaped==failure && b.escaped==failure);
        require(a.effects.equals(failure==null ? "argument;value;after;" : "argument;value;"));
        require(a.effects.equals(b.effects));
        require(b.entries==(failure instanceof Exception ? 1 : 0));
    }
    static void finallyCase(Throwable body, Throwable cleanup, Throwable expected) {
        primary=body; cleanupFailure=cleanup;
        StackTraceElement[] stack=expected==null ? null : expected.getStackTrace();
        Observation a=observe(CheckedBridgeProbe::baselineFinally);
        Observation b=observe(CheckedBridgeProbe::bridgedFinally);
        require(a.escaped==expected && b.escaped==expected);
        require(a.effects.equals("argument;invoke;cleanup;") && a.effects.equals(b.effects));
        if(expected!=null) require(Arrays.equals(stack,expected.getStackTrace()));
    }
    static void suppressedCase(boolean bridged) {
        IOException p=new IOException("primary");
        IOException s=new IOException("secondary");
        primary=p; cleanupFailure=s;
        StackTraceElement[] stack=p.getStackTrace();
        Observation o=observe(bridged ? CheckedBridgeProbe::bridgedSuppressed : CheckedBridgeProbe::baselineSuppressed);
        require(o.escaped==p);
        require(p.getSuppressed().length==1 && p.getSuppressed()[0]==s);
        require(Arrays.equals(stack,p.getStackTrace()));
        require(p.getMessage().equals("primary"));
        require(o.effects.equals("argument;invoke;cleanup;"));
    }
    // A bridge must not move original cleanup into the body catch extent.
    static void originalScope() throws Exception {
        try { effects.append("body;"); }
        catch(RuntimeException original) { cleanup(); throw original; }
        cleanup();
    }
    static void correctScopeBridge() {
        try { effects.append("body;"); }
        catch(RuntimeException original) {
            try { cleanup(); } catch(Exception failure) { CheckedBridgeProbe.<RuntimeException>rethrow(failure); }
            throw original;
        }
        try { cleanup(); } catch(Exception failure) { CheckedBridgeProbe.<RuntimeException>rethrow(failure); }
    }
    static void unsoundCast(Throwable failure) { throw (RuntimeException) failure; }
    public static void main(String[] args) {
        invocation(null,false);
        invocation(new IOException("checked"),true);
        invocation(new RuntimeException("unchecked"),true);
        invocation(new AssertionError("error"),false);
        // Preconstructed OOME tests thrown-object passthrough, not allocation exhaustion.
        invocation(new OutOfMemoryError("synthetic OOME"),false);
        argumentOrdering(new IOException("argument checked"),true);
        argumentOrdering(new AssertionError("argument error"),false);
        valueCase(null); valueCase(new IOException("value checked"));
        valueCase(new RuntimeException("value unchecked")); valueCase(new AssertionError("value error"));
        IOException p=new IOException("body"), s=new IOException("cleanup");
        finallyCase(p,null,p); finallyCase(p,s,s); finallyCase(null,s,s);
        AssertionError e=new AssertionError("cleanup error");
        finallyCase(p,e,e); finallyCase(null,null,null);
        suppressedCase(false); suppressedCase(true);
        primary=null; cleanupFailure=new IOException("outside");
        Observation a=observe(CheckedBridgeProbe::originalScope);
        Observation b=observe(CheckedBridgeProbe::correctScopeBridge);
        require(a.escaped==cleanupFailure && b.escaped==cleanupFailure);
        require(a.effects.equals("body;cleanup;") && a.effects.equals(b.effects));
        IOException checked=new IOException("not a RuntimeException");
        Observation wrong=observe(() -> unsoundCast(checked));
        require(wrong.escaped instanceof ClassCastException && wrong.escaped!=checked);
        System.out.println("checks="+checks+" identity/order/stack/suppression/cleanup/error=passed");
    }
}
'''


def run(argv):
    result = subprocess.run(argv, text=True, capture_output=True, check=False)
    return {"argv": [str(x) for x in argv], "exit": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}


def main():
    root = pathlib.Path(tempfile.mkdtemp(prefix="checked-source-bridge-wave31-", dir="/tmp/opencode"))
    source = root / "CheckedBridgeProbe.java"
    source.write_text(SOURCE)
    results = {"scope": "standalone generated fixture; no original methods or APK exports",
               "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(), "jdks": {}}
    jdks = {"17": pathlib.Path("/home/jtjisgod/tools/jdk-17.0.13+11"),
            "25": pathlib.Path("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1")}
    for release, jdk in jdks.items():
        out = root / release
        out.mkdir()
        version = run([jdk / "bin/java", "-version"])
        compile_result = run([jdk / "bin/javac", "--release", release, "-d", out, source])
        assert compile_result["exit"] == 0, compile_result
        execution = run([jdk / "bin/java", "-cp", out, "CheckedBridgeProbe"])
        assert execution["exit"] == 0, execution
        disassembly = run([jdk / "bin/javap", "-classpath", out, "-p", "-c", "-s", "-v", "CheckedBridgeProbe"])
        assert disassembly["exit"] == 0, disassembly
        text = disassembly["stdout"]
        start = text.index("private static <T extends java.lang.Throwable> void rethrow")
        end = text.index("static void raise", start)
        helper = text[start:end]
        assert "descriptor: (Ljava/lang/Throwable;)V" in helper
        assert "athrow" in helper
        # T erases to Throwable: no RuntimeException checkcast is inserted.
        assert "checkcast" not in helper
        value_start = helper.index("private static <T extends java.lang.Throwable> java.lang.RuntimeException rethrowValue")
        value_helper = helper[value_start:]
        assert "descriptor: (Ljava/lang/Throwable;)Ljava/lang/RuntimeException;" in value_helper
        assert "athrow" in value_helper
        negative = root / "VoidBridgeDefiniteAssignment.java"
        negative.write_text('''class VoidBridgeDefiniteAssignment {
          @SuppressWarnings("unchecked") static <T extends Throwable> void raise(Throwable e) throws T {throw (T)e;}
          static Object invoke() throws Exception {return null;}
          static Object caller(){Object result; try{result=invoke();} catch(Exception e){VoidBridgeDefiniteAssignment.<RuntimeException>raise(e);} return result;}
        }''')
        negative_compile = run([jdk / "bin/javac", "--release", release, "-d", out, negative])
        assert negative_compile["exit"] != 0
        assert "might not have been initialized" in negative_compile["stderr"]
        (out / "javap.txt").write_text(text)
        (out / "rethrow-bytecode.txt").write_text(helper)
        tools = {name: {"path": str(jdk / "bin" / name),
                         "sha256": hashlib.sha256((jdk / "bin" / name).read_bytes()).hexdigest()}
                 for name in ("java", "javac", "javap")}
        results["jdks"][release] = {"tools": tools, "version": version, "compile": compile_result,
                                     "negative_void_definite_assignment": negative_compile,
                                     "run": execution, "rethrow_bytecode": helper,
                                     "class_sha256": hashlib.sha256((out / "CheckedBridgeProbe.class").read_bytes()).hexdigest()}
    results["limits"] = ["Constructor-prechain excluded; no uninitialized receiver use.",
                         "No original recovery identities; synthetic helper tracked separately.",
                         "Preconstructed OOME passthrough only; real memory exhaustion not tested.",
                         "This probe does not attest original APK dispatch/access or metadata.",
                         "No global throws erasure: original runtime escaping domain must remain recorded.",
                         "Generated helpers add source/binary members; reflection and live stack observability differ."]
    (root / "report.json").write_text(json.dumps(results, indent=2))
    print(root)
    for release, result in results["jdks"].items():
        print(release, result["run"]["stdout"].strip())


if __name__ == "__main__":
    main()
