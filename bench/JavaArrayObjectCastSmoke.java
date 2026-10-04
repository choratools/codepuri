package androidx.constraintlayout.core.motion.utils;

import androidx.camera.core.FocusMeteringResult;
import androidx.compose.runtime.IntStack;
import androidx.compose.runtime.external.kotlinx.collections.immutable.internal.DeltaCounter;

/** Executes unchanged representative source units exported from OKcashbag. */
public final class JavaArrayObjectCastSmoke {
    public static void main(String[] args) {
        TypedBundle bundle = new TypedBundle();
        if (bundle.mValueInt.length != 10 || bundle.mValueFloat.length != 10
                || bundle.mValueString.length != 5 || bundle.mValueBoolean.length != 4
                || bundle.mValueInt[0] != 0 || bundle.mValueFloat[0] != 0.0f
                || bundle.mValueString[0] != null || bundle.mValueBoolean[0]) {
            throw new AssertionError("array constructor shape/defaults");
        }
        bundle.mCountInt = 3;
        bundle.clear();
        if (bundle.mCountInt != 0) throw new AssertionError("clear");
        IntStack stack = new IntStack();
        if (stack.getSize() != 0 || stack.peek(0) != 0) {
            throw new AssertionError("array allocation/load");
        }
        try {
            stack.peek(10);
            throw new AssertionError("missing array bounds check");
        } catch (ArrayIndexOutOfBoundsException expected) {
            // Expected Java array semantics.
        }
        if (!FocusMeteringResult.create(true).isFocusSuccessful()
                || FocusMeteringResult.create(false).isFocusSuccessful()) {
            throw new AssertionError("exact boolean constructor argument");
        }
        DeltaCounter original = new DeltaCounter(7);
        DeltaCounter copy = original.copy(-3);
        copy.plusAssign(2);
        if (original.getCount() != 7 || copy.getCount() != -1 || original == copy) {
            throw new AssertionError("constructor descriptor, identity, and state");
        }
        System.out.println("PASS unchanged OKcashbag array/object source units");
    }
}
