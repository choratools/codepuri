import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
public class Builder implements Opcodes {
 public static void main(String[] args)throws Exception {
  String owner="com/samsung/android/sdk/mobileservice/util/SdkLog", arr="[L"+owner+";";
  ClassWriter c=new ClassWriter(ClassWriter.COMPUTE_MAXS);
  c.visit(V17,ACC_PUBLIC|ACC_FINAL|ACC_ENUM|ACC_SUPER,owner,"Ljava/lang/Enum<L"+owner+";>;","java/lang/Enum",null);
  c.visitField(ACC_PRIVATE|ACC_STATIC|ACC_FINAL|ACC_SYNTHETIC,"$VALUES",arr,null,null).visitEnd();
  c.visitField(ACC_PRIVATE|ACC_STATIC|ACC_FINAL,"ENG","Z",null,null).visitEnd();
  c.visitField(ACC_PUBLIC|ACC_STATIC|ACC_FINAL|ACC_ENUM,"INSTANCE","L"+owner+";",null,null).visitEnd();
  c.visitField(ACC_PRIVATE|ACC_STATIC|ACC_FINAL,"LINE_LENGTH","I",null,512).visitEnd();
  c.visitField(ACC_PRIVATE|ACC_STATIC|ACC_FINAL,"prefix","Ljava/lang/String;",null,"SEMS-11.1.04_").visitEnd();
  c.visitField(ACC_PRIVATE|ACC_STATIC|ACC_FINAL,"version","Ljava/lang/String;",null,"11.1.04_").visitEnd();
  MethodVisitor m=c.visitMethod(ACC_PRIVATE,"<init>","(Ljava/lang/String;I)V",null,null);m.visitCode();m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,1);m.visitVarInsn(ILOAD,2);m.visitMethodInsn(INVOKESPECIAL,"java/lang/Enum","<init>","(Ljava/lang/String;I)V",false);m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();
  m=c.visitMethod(ACC_STATIC,"<clinit>","()V",null,null);m.visitCode();
  m.visitTypeInsn(NEW,owner);m.visitInsn(DUP);m.visitLdcInsn("INSTANCE");m.visitInsn(ICONST_0);m.visitMethodInsn(INVOKESPECIAL,owner,"<init>","(Ljava/lang/String;I)V",false);m.visitVarInsn(ASTORE,0);
  m.visitVarInsn(ALOAD,0);m.visitFieldInsn(PUTSTATIC,owner,"INSTANCE","L"+owner+";");
  m.visitInsn(ICONST_1);m.visitTypeInsn(ANEWARRAY,owner);m.visitInsn(DUP);m.visitInsn(ICONST_0);m.visitVarInsn(ALOAD,0);m.visitInsn(AASTORE);m.visitFieldInsn(PUTSTATIC,owner,"$VALUES",arr);
  m.visitLdcInsn("eng");m.visitFieldInsn(GETSTATIC,"android/os/Build","TYPE","Ljava/lang/String;");m.visitMethodInsn(INVOKEVIRTUAL,"java/lang/String","equals","(Ljava/lang/Object;)Z",false);m.visitFieldInsn(PUTSTATIC,owner,"ENG","Z");m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();
  m=c.visitMethod(ACC_PUBLIC|ACC_STATIC,"values","()"+arr,null,null);m.visitCode();m.visitFieldInsn(GETSTATIC,owner,"$VALUES",arr);m.visitMethodInsn(INVOKEVIRTUAL,arr,"clone","()Ljava/lang/Object;",false);m.visitTypeInsn(CHECKCAST,arr);m.visitInsn(ARETURN);m.visitMaxs(0,0);m.visitEnd();
  m=c.visitMethod(ACC_PUBLIC|ACC_STATIC,"valueOf","(Ljava/lang/String;)L"+owner+";",null,null);m.visitCode();m.visitLdcInsn(Type.getObjectType(owner));m.visitVarInsn(ALOAD,0);m.visitMethodInsn(INVOKESTATIC,"java/lang/Enum","valueOf","(Ljava/lang/Class;Ljava/lang/String;)Ljava/lang/Enum;",false);m.visitTypeInsn(CHECKCAST,owner);m.visitInsn(ARETURN);m.visitMaxs(0,0);m.visitEnd();
  c.visitEnd();Path file=Path.of(args[0],owner+".class");Files.createDirectories(file.getParent());Files.write(file,c.toByteArray());
 }
}
