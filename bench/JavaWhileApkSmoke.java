import com.nbt.oss.exp4j.function.Function;

/** Run against the actual exported OKcashbag Function.java, without stubs. */
public final class JavaWhileApkSmoke {
    public static void main(String[] args) {
        String expected = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_";
        for (int i = 0; i < 3; i++) {
            char[] actual = Function.getAllowedFunctionCharacters();
            if (!expected.equals(new String(actual))) {
                throw new AssertionError(new String(actual));
            }
        }
        System.out.println("APK sequential while loops: 3 exact alphabet checks passed");
    }
}
