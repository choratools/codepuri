import java.nio.file.*;
import java.util.*;
import jdk.internal.org.objectweb.asm.*;
// Test oracle only. The reference models the original DEX encoded-value
// installation boundary before the original enum allocations. It does not
// attest equivalence of DEX and JVM execution or native/reflection behavior.
public class PrepareCapture {
 private static final String OWNER="com/samsung/android/samsungpass/ExportFailureState";
 private static final Map<String,Integer> PREPARED=Map.of(
  "EXPORT_FAILURE_STATE_UNSPECIFIED_VALUE",0,
  "EXPORT_FAILURE_STATE_UNTRANSFERABLE_VALUE",1,
  "EXPORT_FAILURE_STATE_EXPORT_FAILED_VALUE",2);
 public static void main(String[] args) throws Exception {
  Path target=Path.of(args[0]);
  boolean reference=args.length==2 && args[1].equals("reference");
  ClassReader reader=new ClassReader(Files.readAllBytes(target));
  ClassWriter writer=new ClassWriter(reader,ClassWriter.COMPUTE_MAXS);
  final int[] constructors={0},fields={0},getNumber={0},clinit={0};
  reader.accept(new ClassVisitor(Opcodes.ASM8,writer) {
   @Override public void visit(int version,int access,String name,String signature,String parent,String[] interfaces) {
    if(!name.equals(OWNER) || !parent.equals("java/lang/Enum")
      || (access & (Opcodes.ACC_PUBLIC|Opcodes.ACC_FINAL|Opcodes.ACC_ENUM|Opcodes.ACC_INTERFACE|Opcodes.ACC_ABSTRACT))
      != (Opcodes.ACC_PUBLIC|Opcodes.ACC_FINAL|Opcodes.ACC_ENUM))
     throw new AssertionError("original source enum modifiers/hierarchy changed");
    super.visit(version,access,name,signature,parent,interfaces);
   }
   @Override public FieldVisitor visitField(int access,String name,String desc,String signature,Object value) {
    if(PREPARED.containsKey(name)) {
     fields[0]++;
     if(!desc.equals("I") || access!=(Opcodes.ACC_PUBLIC|Opcodes.ACC_STATIC|Opcodes.ACC_FINAL)
       || !Objects.equals(value,PREPARED.get(name)))
      throw new AssertionError("original encoded value/field modifiers changed: "+name);
     if(reference) value=null;
    }
    return super.visitField(access,name,desc,signature,value);
   }
   @Override public MethodVisitor visitMethod(int access,String name,String desc,String sig,String[] exceptions) {
    MethodVisitor delegate=super.visitMethod(access,name,desc,sig,exceptions);
    if(name.equals("getNumber") && desc.equals("()I")) {
     getNumber[0]++;
     if(access!=(Opcodes.ACC_PUBLIC|Opcodes.ACC_FINAL))
      throw new AssertionError("original final getNumber modifiers changed");
    }
    if(name.equals("<clinit>") && desc.equals("()V")) {
     clinit[0]++;
     if(!reference) return delegate;
     return new MethodVisitor(Opcodes.ASM8,delegate) {
      @Override public void visitCode() {
       super.visitCode();
       for(String field: List.of("EXPORT_FAILURE_STATE_UNSPECIFIED_VALUE",
           "EXPORT_FAILURE_STATE_UNTRANSFERABLE_VALUE","EXPORT_FAILURE_STATE_EXPORT_FAILED_VALUE")) {
        visitLdcInsn(PREPARED.get(field));
        visitFieldInsn(Opcodes.PUTSTATIC,OWNER,field,"I");
       }
      }
     };
    }
    if(!name.equals("<init>") || !desc.equals("(Ljava/lang/String;II)V")) return delegate;
    constructors[0]++;
    if(access!=Opcodes.ACC_PRIVATE) throw new AssertionError("original constructor privacy changed");
    return new MethodVisitor(Opcodes.ASM8,delegate) {
     @Override public void visitCode() {
      super.visitCode();
      visitLdcInsn(Type.getObjectType(OWNER));
      visitMethodInsn(Opcodes.INVOKESTATIC,"com/samsung/android/samsungpass/PreparedRecorder","record","(Ljava/lang/Class;)V",false);
     }
    };
   }
  },0);
  if(constructors[0]!=1 || fields[0]!=3 || getNumber[0]!=1 || clinit[0]!=1)
   throw new AssertionError("exact original declarations absent");
  byte[] bytes=writer.toByteArray();
  final int[] constantValues={0},originalStores={0};
  new ClassReader(bytes).accept(new ClassVisitor(Opcodes.ASM8) {
   @Override public FieldVisitor visitField(int access,String name,String desc,String signature,Object value) {
    if(PREPARED.containsKey(name) && value!=null) constantValues[0]++;
    return null;
   }
   @Override public MethodVisitor visitMethod(int access,String name,String desc,String sig,String[] exceptions) {
    if(!name.equals("<clinit>")) return null;
    return new MethodVisitor(Opcodes.ASM8) {
     @Override public void visitFieldInsn(int opcode,String owner,String name,String descriptor) {
      if(opcode==Opcodes.PUTSTATIC && owner.equals(OWNER) && PREPARED.containsKey(name)) originalStores[0]++;
     }
    };
   }
  },ClassReader.SKIP_DEBUG|ClassReader.SKIP_FRAMES);
  if(constantValues[0]!=(reference?0:3) || originalStores[0]!=(reference?3:0))
   throw new AssertionError("distinct JVM boundary not represented");
  Files.write(target,bytes);
  System.out.println("BOUNDARY "+(reference?"reference entry stores":"emitted ConstantValue")+
   " constants="+constantValues[0]+" stores="+originalStores[0]);
 }
}
