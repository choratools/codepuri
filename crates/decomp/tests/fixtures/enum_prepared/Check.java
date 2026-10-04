package oracle;
import java.lang.reflect.*;
public class Check {
 public static final Error FAILURE=new Error("same");
 public static String trace="";
 public static final String OWNER="com.samsung.android.sdk.mobileservice.util.SdkLog";
 public static Object get(String name)throws Exception {Class<?> c=Class.forName(OWNER,false,Check.class.getClassLoader());Field f=c.getDeclaredField(name);f.setAccessible(true);return f.get(null);}
 public static void boundary() {
  try {
   trace=get("LINE_LENGTH")+":"+get("prefix")+":"+get("version")+":"+get("INSTANCE")+":"+get("ENG");
   if(System.getProperty("kind").equals("mutate")) {
    Field uf=sun.misc.Unsafe.class.getDeclaredField("theUnsafe");uf.setAccessible(true);sun.misc.Unsafe u=(sun.misc.Unsafe)uf.get(null);
    Field f=Class.forName(OWNER,false,Check.class.getClassLoader()).getDeclaredField("prefix");u.putObject(u.staticFieldBase(f),u.staticFieldOffset(f),"modified");
   }
   if(System.getProperty("kind").equals("fail")) throw FAILURE;
  }catch(Error e){throw e;}catch(Exception e){throw new AssertionError(e);}
 }
 public static void main(String[] args)throws Exception {
  System.setProperty("kind",args[0]);
  try {
   Class<?> c=Class.forName(OWNER);Object[] values=(Object[])c.getMethod("values").invoke(null);Object instance=values[0];values[0]=null;
   if(((Object[])c.getMethod("values").invoke(null))[0]!=instance||c.getMethod("valueOf",String.class).invoke(null,"INSTANCE")!=instance||!((Enum<?>)instance).name().equals("INSTANCE")||((Enum<?>)instance).ordinal()!=0)throw new AssertionError();
   System.out.print(trace+"|"+get("ENG")+"|"+get("prefix"));
  }catch(Error error){if(error!=FAILURE)throw error;System.out.print(trace+"|same");}
 }
}
