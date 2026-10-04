import java.nio.file.*;import jdk.internal.org.objectweb.asm.*;
public final class Builder implements Opcodes {
 static ClassWriter clazz(String n){ClassWriter w=new ClassWriter(3);w.visit(V17,ACC_PUBLIC|ACC_SUPER,n,null,"java/lang/Object",null);return w;}
 public static void main(String[] a)throws Exception{
  for(String name:new String[]{"Big","Inner"}){ClassWriter w=clazz(name);boolean outer=name.equals("Big");
   if(outer)for(String field:new String[]{"first","second","third"})w.visitField(ACC_PUBLIC|ACC_FINAL,field,"Ljava/lang/Object;",null,null).visitEnd();
   for(int n=0;n<(outer?16000:60000);n++)w.visitField(ACC_PUBLIC,"pad"+n,"J",null,null).visitEnd();
   MethodVisitor m=w.visitMethod(ACC_STATIC,"<clinit>","()V",null,null);m.visitCode();m.visitLdcInsn(outer?"OC":"IC");m.visitMethodInsn(INVOKESTATIC,"Probe","event","(Ljava/lang/String;)V",false);m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();
   m=w.visitMethod(ACC_PUBLIC,"<init>",outer?"(Ljava/lang/Object;Ljava/lang/String;Ljava/lang/Object;)V":"()V",null,null);m.visitCode();m.visitVarInsn(ALOAD,0);m.visitMethodInsn(INVOKESPECIAL,"java/lang/Object","<init>","()V",false);m.visitLdcInsn(outer?"OT":"IT");m.visitMethodInsn(INVOKESTATIC,"Probe","event","(Ljava/lang/String;)V",false);
   if(outer)for(int n=1;n<=3;n++){m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,n);m.visitFieldInsn(PUTFIELD,name,new String[]{"first","second","third"}[n-1],"Ljava/lang/Object;");}m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();w.visitEnd();Files.write(Path.of(a[0],name+".class"),w.toByteArray());
  }
  ClassWriter w=clazz("Original");MethodVisitor m=w.visitMethod(ACC_PUBLIC|ACC_STATIC,"make","()LBig;",null,null);m.visitCode();m.visitTypeInsn(NEW,"Big");m.visitInsn(DUP);m.visitTypeInsn(NEW,"Inner");m.visitInsn(DUP);m.visitMethodInsn(INVOKESPECIAL,"Inner","<init>","()V",false);m.visitVarInsn(ASTORE,0);
  m.visitVarInsn(ALOAD,0);m.visitMethodInsn(INVOKESTATIC,"Probe","touch","(LInner;)V",false);m.visitVarInsn(ALOAD,0);m.visitMethodInsn(INVOKESTATIC,"Probe","value","(LInner;)Ljava/lang/Object;",false);m.visitVarInsn(ASTORE,1);
  m.visitMethodInsn(INVOKESTATIC,"Probe","file","()Ljava/lang/Object;",false);m.visitVarInsn(ASTORE,2);m.visitLdcInsn("late");m.visitVarInsn(ASTORE,3);
  m.visitVarInsn(ALOAD,1);m.visitVarInsn(ALOAD,3);m.visitVarInsn(ALOAD,2);m.visitMethodInsn(INVOKESPECIAL,"Big","<init>","(Ljava/lang/Object;Ljava/lang/String;Ljava/lang/Object;)V",false);m.visitInsn(ARETURN);m.visitMaxs(0,0);m.visitEnd();w.visitEnd();Files.write(Path.of(a[0],"Original.class"),w.toByteArray());
 }
}
