public final class Harness {
    public static void main(String[] args) throws Exception {
        boolean oracle = args[0].equals("Original");
        String mode = args[1];
        Object parameter = mode.endsWith("reference") ? new Object() : null;
        Probe.failClass = mode.startsWith("classfail");
        Probe.failConstructor = mode.startsWith("ctorfail");
        boolean oom = mode.startsWith("oom");
        if (oom) {
            // Resolve both method bodies and initialize Big before filling the
            // heap, so the measured failure is the actual large NEW operation.
            Original.make(parameter);
            Emitted.make(parameter);
            Probe.trace.setLength(0); Probe.constructions = 0;
            Probe.retained = new byte[30000][];
            int count = 0;
            try {
                while (count < Probe.retained.length) {
                    Probe.retained[count] = new byte[4096]; count++;
                }
                throw new AssertionError("heap was not exhausted");
            } catch (OutOfMemoryError expected) {
                // Retain every successfully allocated array until after NEW.
            }
        }
        Big value = null; Throwable failure = null;
        try {
            value = oracle ? Original.make(parameter) : Emitted.make(parameter);
        } catch (Throwable caught) {
            failure = caught;
        }
        if (oom) {
            if (!(failure instanceof OutOfMemoryError) || Probe.constructions != 0) {
                Probe.retained = null;
                throw new AssertionError("expected genuine pre-constructor allocation OOM");
            }
            Probe.retained = null;
            System.gc();
            System.out.println("OOM:constructor-not-entered");
        } else if (mode.startsWith("classfail")) {
            if (failure != Probe.classFailure || Probe.constructions != 0) throw new AssertionError("class exception identity");
            System.out.println(Probe.trace + ":class-failure-identity");
        } else if (mode.startsWith("ctorfail")) {
            if (failure != Probe.constructorFailure || Probe.constructions != 1) throw new AssertionError("constructor exception identity");
            System.out.println(Probe.trace + ":constructor-failure-identity");
        } else {
            if (failure != null || value == null || value.value != parameter || value.tag != (parameter == null ? 0 : 1)) throw new AssertionError("selected constructor/argument identity");
            System.out.println(Probe.trace + ":" + value.tag + ":argument-identity");
        }
    }
}
