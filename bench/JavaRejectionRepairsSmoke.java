import androidx.camera.core.CameraState$Type;
import androidx.camera.core.impl.Timebase;

/** Runtime assertions against unchanged retained APK units, with no Android calls. */
public final class JavaRejectionRepairsSmoke {
    public static void main(String[] args) {
        String[] names = {"PENDING_OPEN", "OPENING", "OPEN", "CLOSING", "CLOSED"};
        CameraState$Type[] values = CameraState$Type.values();
        for (int i = 0; i < names.length; i++) {
            if (!names[i].equals(values[i].name()) || values[i].ordinal() != i
                    || CameraState$Type.valueOf(names[i]) != values[i]) throw new AssertionError(names[i]);
        }
        values[0] = null;
        if (CameraState$Type.values()[0] == null) throw new AssertionError("values array alias");
        if (Timebase.values().length != 2 || Timebase.valueOf("REALTIME").ordinal() != 1)
            throw new AssertionError("timebase");
        System.out.println("PASS unchanged final APK enum names/order/ordinals/valueOf/fresh arrays");
    }
}
