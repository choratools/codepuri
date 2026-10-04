public final class Probe {
    static final Error classFailure = new Error("class-init-identity");
    static final RuntimeException constructorFailure = new RuntimeException("constructor-identity");
    static final StringBuilder trace = new StringBuilder();
    static boolean failClass, failConstructor;
    static int constructions;
    static byte[][] retained;
    public static void classInit() {
        trace.append('C');
        if (failClass) throw classFailure;
    }
    public static void construct(int arity) {
        trace.append('T').append(arity);
        constructions++;
        if (failConstructor) throw constructorFailure;
    }
}
