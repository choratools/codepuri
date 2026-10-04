public final class Probe {
 static final Object token=new Object(),other=new Object();
 static final Error classFailure=new Error("class-init");
 static final RuntimeException failure=new RuntimeException("exact-original-exception");
 static String mode="";static StringBuilder trace=new StringBuilder(256);static int calls,constructions;static byte[][] retained;static byte[] reserve;static int touches;
 static void event(String name){trace.append(name);if(mode.equals(name))throw failure;}
 static void classInit(){trace.append("C");if(mode.equals("class"))throw classFailure;}
 public static Object first(){event("F");return token;}
 public static Object next(){calls++;event("N"+calls);return mode.equals("null")||mode.equals("alternate")&&calls==2?null:other;}
 public static Object extra(Object value){event("E");if(value==null)throw failure;return value;}
 static void innerClassInit(){trace.append("K");if(mode.equals("innerclass"))throw classFailure;}
 static void innerConstruct(){event("I");}
 static void touch(){event("U"+(++touches));}
 static void construct(){constructions++;event("T");}
}
