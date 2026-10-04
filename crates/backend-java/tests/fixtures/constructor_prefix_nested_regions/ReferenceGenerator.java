import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
public final class ReferenceGenerator implements Opcodes {
 public static void main(String[] args) throws Exception {
  ClassWriter c=new ClassWriter(ClassWriter.COMPUTE_FRAMES|ClassWriter.COMPUTE_MAXS);
  c.visit(V17,ACC_PUBLIC,"FixtureOriginal",null,"Base",null);
  MethodVisitor m=c.visitMethod(ACC_PUBLIC,"<init>","(Z)V",null,null);m.visitCode();
  m.visitMethodInsn(INVOKESTATIC,"Trace","token","()I",false);m.visitVarInsn(ISTORE,2);
  for(int stage=0;stage<2;stage++) {
   Label fallback=new Label(), selected=new Label();
   m.visitVarInsn(ILOAD,1);m.visitJumpInsn(IFEQ,fallback);
   m.visitMethodInsn(INVOKESTATIC,"Trace",stage==0?"get1":"get2","()I",false);
   m.visitVarInsn(ISTORE,3+stage);m.visitVarInsn(ILOAD,3+stage);m.visitJumpInsn(IFNE,selected);
   m.visitLabel(fallback);m.visitMethodInsn(INVOKESTATIC,"Trace",stage==0?"fallback1":"fallback2","()I",false);
   m.visitVarInsn(ISTORE,3+stage);m.visitLabel(selected);
  }
  m.visitVarInsn(ALOAD,0);m.visitVarInsn(ILOAD,2);m.visitVarInsn(ILOAD,3);m.visitVarInsn(ILOAD,4);
  m.visitMethodInsn(INVOKESPECIAL,"Base","<init>","(III)V",false);m.visitInsn(RETURN);
  m.visitMaxs(0,0);m.visitEnd();c.visitEnd();Files.write(Paths.get("FixtureOriginal.class"),c.toByteArray());
 }
}
