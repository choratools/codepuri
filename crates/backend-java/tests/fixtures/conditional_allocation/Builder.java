import java.nio.file.Files;
import java.nio.file.Path;
import jdk.internal.org.objectweb.asm.*;

/** Verifier-valid original NEW/DUP before the null branch, not javac's output. */
public final class Builder implements Opcodes {
    private static ClassWriter clazz(String name) {
        ClassWriter out = new ClassWriter(ClassWriter.COMPUTE_FRAMES | ClassWriter.COMPUTE_MAXS);
        out.visit(V17, ACC_PUBLIC | ACC_SUPER, name, null, "java/lang/Object", null);
        return out;
    }
    public static void main(String[] args) throws Exception {
        Path root = Path.of(args[0]);
        ClassWriter big = clazz("Big");
        big.visitField(ACC_PUBLIC | ACC_FINAL, "tag", "I", null, null).visitEnd();
        big.visitField(ACC_PUBLIC | ACC_FINAL, "value", "Ljava/lang/Object;", null, null).visitEnd();
        // Force a real NEW allocation failure under -Xmx16m after the harness
        // retains tiny arrays. This is not a constructor that throws a fake OOM.
        for (int i = 0; i < 16000; i++) {
            big.visitField(ACC_PUBLIC, "pad" + i, "J", null, null).visitEnd();
        }
        MethodVisitor mv = big.visitMethod(ACC_STATIC, "<clinit>", "()V", null, null);
        mv.visitCode();
        mv.visitMethodInsn(INVOKESTATIC, "Probe", "classInit", "()V", false);
        mv.visitInsn(RETURN); mv.visitMaxs(0, 0); mv.visitEnd();
        for (int arity = 0; arity < 2; arity++) {
            mv = big.visitMethod(ACC_PUBLIC, "<init>", arity == 0 ? "()V" : "(Ljava/lang/Object;)V", null, null);
            mv.visitCode();
            mv.visitVarInsn(ALOAD, 0);
            mv.visitMethodInsn(INVOKESPECIAL, "java/lang/Object", "<init>", "()V", false);
            mv.visitInsn(arity == 0 ? ICONST_0 : ICONST_1);
            mv.visitMethodInsn(INVOKESTATIC, "Probe", "construct", "(I)V", false);
            mv.visitVarInsn(ALOAD, 0); mv.visitInsn(arity == 0 ? ICONST_0 : ICONST_1);
            mv.visitFieldInsn(PUTFIELD, "Big", "tag", "I");
            mv.visitVarInsn(ALOAD, 0);
            if (arity == 0) mv.visitInsn(ACONST_NULL); else mv.visitVarInsn(ALOAD, 1);
            mv.visitFieldInsn(PUTFIELD, "Big", "value", "Ljava/lang/Object;");
            mv.visitInsn(RETURN); mv.visitMaxs(0, 0); mv.visitEnd();
        }
        big.visitEnd(); Files.write(root.resolve("Big.class"), big.toByteArray());
        ClassWriter original = clazz("Original");
        mv = original.visitMethod(ACC_PUBLIC | ACC_STATIC, "make", "(Ljava/lang/Object;)LBig;", null, null);
        mv.visitCode();
        mv.visitTypeInsn(NEW, "Big");
        mv.visitInsn(DUP);
        mv.visitVarInsn(ALOAD, 0);
        Label noArgument = new Label(), done = new Label();
        mv.visitJumpInsn(IFNULL, noArgument);
        mv.visitVarInsn(ALOAD, 0);
        mv.visitMethodInsn(INVOKESPECIAL, "Big", "<init>", "(Ljava/lang/Object;)V", false);
        mv.visitJumpInsn(GOTO, done);
        mv.visitLabel(noArgument);
        mv.visitMethodInsn(INVOKESPECIAL, "Big", "<init>", "()V", false);
        mv.visitLabel(done); mv.visitInsn(ARETURN);
        mv.visitMaxs(0, 0); mv.visitEnd();
        original.visitEnd(); Files.write(root.resolve("Original.class"), original.toByteArray());
    }
}
