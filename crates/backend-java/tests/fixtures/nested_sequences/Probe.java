public final class Probe {
 static String mode="";static final Object first=new Object(),third=new Object();static final Error initFailure=new Error("clinit");static final RuntimeException failure=new RuntimeException("exact");static StringBuilder trace=new StringBuilder(128);static byte[][] retained;
 public static void event(String name){trace.append(name);if(mode.equals(name)){if(name.endsWith("C"))throw initFailure;throw failure;}}
 public static void touch(Inner i){event("A");}
 public static Object value(Inner i){event("V");return first;}
 public static Object file(){event("F");return third;}
}
