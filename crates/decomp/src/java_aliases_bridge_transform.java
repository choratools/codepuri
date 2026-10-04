import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
public class RenameOriginal {
 public static void main(String[]args)throws Exception{
  for(Path p:Files.newDirectoryStream(Path.of(args[0]),"*.class")){
   ClassWriter w=new ClassWriter(0);
   new ClassReader(Files.readAllBytes(p)).accept(new ClassVisitor(Opcodes.ASM8,w){
    public MethodVisitor visitMethod(int access,String name,String desc,String signature,String[]exceptions){
     boolean bridge=name.equals("create");
     boolean typed=name.equals("create$typed");
     if(bridge && !p.getFileName().toString().equals("Wire.class") && !p.getFileName().toString().equals("GrandChild.class"))access|=Opcodes.ACC_BRIDGE|Opcodes.ACC_SYNTHETIC;
     MethodVisitor m=super.visitMethod(access,typed?"create":name,desc,signature,exceptions);
     return new MethodVisitor(Opcodes.ASM8,m){public void visitMethodInsn(int op,String owner,String name,String desc,boolean itf){super.visitMethodInsn(op,owner,name.equals("create$typed")?"create":name,desc,itf);}};
    }
   },0);
   Files.write(Path.of(args[1]).resolve(p.getFileName()),w.toByteArray());
  }
 }
}
