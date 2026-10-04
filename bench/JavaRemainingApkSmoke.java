import androidx.camera.core.CameraState$Type;
import androidx.camera.core.impl.Timebase;
import org.apache.commons.lang3.math.c;
import androidx.compose.runtime.IntStack;

/** Executes unchanged production APK units; no source substitutes. */
public class JavaRemainingApkSmoke {
    public static void main(String[] args) throws Exception {
        IntStack stack = new IntStack();
        var slots = IntStack.class.getDeclaredField("slots");
        var tos = IntStack.class.getDeclaredField("tos");
        slots.setAccessible(true);
        tos.setAccessible(true);
        slots.set(stack, new int[] {11, 22, 33, 22, 44});
        tos.setInt(stack, 5);
        if (stack.indexOf(11) != 0 || stack.indexOf(22) != 1
                || stack.indexOf(44) != 4 || stack.indexOf(99) != -1)
            throw new AssertionError("labeled loop Phi, continue, early return, break");
        tos.setInt(stack, 0);
        if (stack.indexOf(11) != -1) throw new AssertionError("zero iteration");
        String[] names = {"PENDING_OPEN", "OPENING", "OPEN", "CLOSING", "CLOSED"};
        CameraState$Type[] values = CameraState$Type.values();
        for (int i = 0; i < names.length; i++) {
            if (!values[i].name().equals(names[i]) || values[i].ordinal() != i
                    || CameraState$Type.valueOf(names[i]) != values[i]) throw new AssertionError();
        }
        values[0] = null;
        if (CameraState$Type.values()[0] == null) throw new AssertionError("values clone");
        if (Timebase.values().length != 2 || Timebase.valueOf("REALTIME").ordinal() != 1)
            throw new AssertionError("timebase");
        for (String text : new String[] {null, "", "bad", "12", "-31", "2147483648"}) {
            int expected;
            try { expected = Integer.parseInt(text); } catch (NumberFormatException e) { expected = 77; }
            if (c.toInt(text, 77) != expected) throw new AssertionError("int " + text);
        }
        if (c.toLong("9223372036854775807", 8) != Long.MAX_VALUE
                || c.toLong("bad", 8) != 8 || c.toLong(null, 8) != 8
                || c.toDouble("-1.25", 8) != -1.25 || c.toDouble("bad", 8) != 8
                || c.toDouble(null, 8) != 8) throw new AssertionError("parse regions");
        System.out.println("PASS unchanged production enums, clone/valueOf/ordinals, pretry and typed catches");
    }
}
