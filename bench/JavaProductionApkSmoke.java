import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;

/** Execute unchanged final APK source units; fixture state is supplied explicitly. */
public final class JavaProductionApkSmoke {
    public static void main(String[] args) throws Exception {
        Class<?> errors = Class.forName("androidx.biometric.ErrorUtils");
        Method known = errors.getDeclaredMethod("isKnownError", int.class);
        known.setAccessible(true);
        for (int i = -10; i <= 25; i++) {
            boolean expected = i >= 1 && i <= 15 && i != 6;
            if (!Boolean.valueOf(expected).equals(known.invoke(null, i))) {
                throw new AssertionError("switch dispatch " + i);
            }
        }
        if (!Integer.valueOf(-1).equals(androidx.annotation.ChecksSdkIntAtLeast.class
                .getMethod("api").getDefaultValue())) {
            throw new AssertionError("annotation api default");
        }
        if (!"".equals(androidx.annotation.ChecksSdkIntAtLeast.class
                .getMethod("codename").getDefaultValue())) {
            throw new AssertionError("annotation string default");
        }
        Class<?> network = Class.forName("com.enliple.keyboard.imgmodule.gifencoder.c");
        Constructor<?> ctor = network.getDeclaredConstructor();
        ctor.setAccessible(true);
        Object instance = ctor.newInstance(); // Explicit fixture construction, not recovered DEX ctor evidence.
        Field colors = network.getDeclaredField("e");
        Field index = network.getDeclaredField("f");
        colors.setAccessible(true);
        index.setAccessible(true);
        int[][] rows = new int[256][4];
        int[] lookup = new int[256];
        for (int i = 0; i < 256; i++) {
            rows[i] = new int[] {i * 3, 255 - i, i * 5, i};
        }
        colors.set(instance, rows);
        index.set(instance, lookup);
        Method build = network.getDeclaredMethod("inxbuild");
        build.setAccessible(true);
        build.invoke(instance);
        for (int green = 0; green < 256; green++) {
            int original = 255 - green;
            int[] expected = {original * 3, green, original * 5, original};
            if (!java.util.Arrays.equals(rows[green], expected) || lookup[green] != green) {
                throw new AssertionError("nested sort/Phi/index " + green);
            }
        }
        System.out.println("PASS unchanged APK grouped/default switch, nested loops, annotation defaults");
    }
}
