import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
public final class Builder implements Opcodes {
 static ClassWriter clazz(String name){ClassWriter w=new ClassWriter(ClassWriter.COMPUTE_FRAMES|ClassWriter.COMPUTE_MAXS);w.visit(V17,ACC_PUBLIC|ACC_SUPER,name,null,"java/lang/Object",null);return w;}
 public static void main(String[] a)throws Exception{
  ClassWriter w=clazz("Big");
  for(String f:new String[]{"first","second"})w.visitField(ACC_PUBLIC|ACC_FINAL,f,"Ljava/lang/Object;",null,null).visitEnd();
  for(int n=0;n<16000;n++)w.visitField(ACC_PUBLIC,"pad"+n,"J",null,null).visitEnd();
  MethodVisitor m=w.visitMethod(ACC_STATIC,"<clinit>","()V",null,null);m.visitCode();m.visitMethodInsn(INVOKESTATIC,"Probe","classInit","()V",false);m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();
  m=w.visitMethod(ACC_PUBLIC,"<init>","(Ljava/lang/Object;Ljava/lang/Object;)V",null,null);m.visitCode();m.visitVarInsn(ALOAD,0);m.visitMethodInsn(INVOKESPECIAL,"java/lang/Object","<init>","()V",false);m.visitMethodInsn(INVOKESTATIC,"Probe","construct","()V",false);
  for(int n=1;n<=2;n++){m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,n);m.visitFieldInsn(PUTFIELD,"Big",n==1?"first":"second","Ljava/lang/Object;");}m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();w.visitEnd();Files.write(Path.of(a[0],"Big.class"),w.toByteArray());
  w=clazz("Original");m=w.visitMethod(ACC_PUBLIC|ACC_STATIC,"make","()LBig;",null,null);m.visitCode();
  m.visitTypeInsn(NEW,"Big");m.visitInsn(DUP);
  m.visitMethodInsn(INVOKESTATIC,"Probe","first","()Ljava/lang/Object;",false);m.visitVarInsn(ASTORE,0);
  m.visitMethodInsn(INVOKESTATIC,"Probe","next","()Ljava/lang/Object;",false);m.visitVarInsn(ASTORE,1);
  m.visitVarInsn(ALOAD,1);Label nil=new Label(),join=new Label();m.visitJumpInsn(IFNULL,nil);
  m.visitMethodInsn(INVOKESTATIC,"Probe","next","()Ljava/lang/Object;",false);m.visitMethodInsn(INVOKESTATIC,"Probe","extra","(Ljava/lang/Object;)Ljava/lang/Object;",false);m.visitJumpInsn(GOTO,join);
  m.visitLabel(nil);m.visitInsn(ACONST_NULL);m.visitLabel(join);m.visitVarInsn(ASTORE,2);
  m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,2);m.visitMethodInsn(INVOKESPECIAL,"Big","<init>","(Ljava/lang/Object;Ljava/lang/Object;)V",false);m.visitInsn(ARETURN);m.visitMaxs(0,0);m.visitEnd();w.visitEnd();Files.write(Path.of(a[0],"Original.class"),w.toByteArray());
 }
}
