#!/usr/bin/env python3
"""Standalone JDK17 fixture: return-only slots and broad-only child dispatch.
Class-file editing changes names, interface identity, and bridge flags only;
original javac-generated method bodies remain untouched. No Samsung bodies.
"""
import pathlib, struct, subprocess, tempfile, json
JDK = pathlib.Path('/home/jtjisgod/tools/jdk-17.0.13+11/bin')

def run(*args):
    p = subprocess.run([str(a) for a in args], capture_output=True, text=True)
    return p.returncode, p.stdout.strip(), p.stderr.strip()

def patch(path, rename, mark_bridge=False):
    data = path.read_bytes()
    count = struct.unpack_from('>H', data, 8)[0]
    cp = [None] * count
    out = bytearray(data[:10]); pos = 10; idx = 1
    sizes = {3:4, 4:4, 5:8, 6:8, 7:2, 8:2, 9:4, 10:4, 11:4, 12:4, 15:3, 16:2, 17:4, 18:4, 19:2, 20:2}
    while idx < count:
        tag = data[pos]; pos += 1; out.append(tag)
        if tag == 1:
            size = struct.unpack_from('>H',data,pos)[0]; pos += 2
            value = data[pos:pos+size]; pos += size
            value = rename.get(value,value); cp[idx] = value
            out.extend(struct.pack('>H',len(value))); out.extend(value)
        else:
            size=sizes[tag]; out.extend(data[pos:pos+size]);pos += size
            if tag in (5,6): idx += 1
        idx += 1
    tail = bytearray(data[pos:])
    if mark_bridge:
        offset = 6
        n=struct.unpack_from('>H',tail,offset)[0]; offset += 2+2*n
        fields=struct.unpack_from('>H',tail,offset)[0];offset += 2
        def skip_member(offset):
            attrs=struct.unpack_from('>H',tail,offset+6)[0];offset += 8
            for _ in range(attrs):
                length=struct.unpack_from('>I',tail,offset+2)[0];offset += 6+length
            return offset
        for _ in range(fields): offset=skip_member(offset)
        methods=struct.unpack_from('>H',tail,offset)[0];offset += 2
        for _ in range(methods):
            flags,name,desc=struct.unpack_from('>HHH',tail,offset)
            if cp[name]==b'c' and cp[desc]==b'(Ljava/lang/String;)Ljava/lang/Object;':
                struct.pack_into('>H',tail,offset,0x1041)
            offset=skip_member(offset)
    out.extend(tail);path.write_bytes(out)

def compile_unit(directory, text):
    directory.mkdir(); source=directory/'Fixture.java';source.write_text(text)
    code,stdout,stderr=run(JDK/'javac','--release','17','-d',directory,source)
    assert code==0,stderr

common='''interface Wide { Object c(String s); }
interface Narrow extends Wide { String c(String s); }
'''
with tempfile.TemporaryDirectory(prefix='alias-broad-child-') as temp:
    root=pathlib.Path(temp); original=root/'original'; renamed=root/'renamed'
    compile_unit(original,common+'''
class Base implements Wide {
 public String narrow(String s) { return "base:"+s; }
 public Object c(String s) { return narrow(s); }
}
class Child extends Base {
 static int effects;
 public Object c(String s) { effects++; return "child:"+s; }
}
class Fixture {
 public static void main(String[] a) {
  Base b=new Child(); Wide w=b;
  System.out.println(w.c("1")+"|"+b.narrow("1")+"|"+Child.effects);
 }
}
''')
    patch(original/'Base.class',{b'narrow':b'c',b'Wide':b'Narrow'},True)
    patch(original/'Fixture.class',{b'narrow':b'c'})
    code,orig,error=run(JDK/'java','-Xverify:all','-cp',original,'Fixture');assert code==0,error
    compile_unit(renamed,common+'''
class Base implements Narrow {
 public String c(String s) { return "base:"+s; }
 public Object c__2(String s) { return c(s); }
}
class Child extends Base {
 static int effects;
 public Object c__2(String s) { effects++; return "child:"+s; }
}
class Fixture {
 public static void main(String[] a) {
  Base b=new Child(); Wide w=b;
  System.out.println(w.c("1")+"|"+b.c("1")+"|"+Child.effects);
 }
}
''')
    code,post,error=run(JDK/'java','-Xverify:all','-cp',renamed,'Fixture');assert code==0,error
    invalid=renamed/'UnrenamedChild.java';invalid.write_text('class UnrenamedChild extends Base { public Object c(String s) { return "effect"; } }')
    code,_,error=run(JDK/'javac','--release','17','-cp',renamed,invalid)
    assert code!=0 and 'return type Object is not compatible with String' in error,error
    assert orig=='child:1|base:1|1',orig
    assert post=='base:1|base:1|0',post
    print(json.dumps({'original_jvm':orig,'propagated_source_aliases':post,'unrenamed_child_javac':'incompatible return type','dispatch_preserved':orig==post,'body_evidence':'JDK17 fixture, original javac method bodies retained'}))
