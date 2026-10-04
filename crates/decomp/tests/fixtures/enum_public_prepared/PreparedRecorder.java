package com.samsung.android.samsungpass;
public class PreparedRecorder {
 public static int calls;
 public static void record(Class<?> owner) {
  try {
   int unspecified=owner.getField("EXPORT_FAILURE_STATE_UNSPECIFIED_VALUE").getInt(null);
   int untransferable=owner.getField("EXPORT_FAILURE_STATE_UNTRANSFERABLE_VALUE").getInt(null);
   int failed=owner.getField("EXPORT_FAILURE_STATE_EXPORT_FAILED_VALUE").getInt(null);
   if(unspecified!=0 || untransferable!=1 || failed!=2) throw new AssertionError("encoded preparation changed");
   calls++;
   System.out.println("PREPARED_CONSTRUCTOR_ENTRY "+unspecified+":"+untransferable+":"+failed);
  } catch(ReflectiveOperationException error) {throw new AssertionError(error);}
 }
}
