#!/usr/bin/env python3
"""Compile unchanged original-method fragments against explicit boundary fixtures.
Not a complete APK/source-closure proof. Existing caller constructor retained.
"""
import hashlib,json,pathlib,subprocess,sys,tempfile
fragments=pathlib.Path(sys.argv[1]);out=pathlib.Path(tempfile.mkdtemp(prefix='checked-source-actual-wave32-',dir='/tmp/opencode'));src=out/'src';src.mkdir()
def write(name,text):
 p=src/(name.replace('.','/')+'.java');p.parent.mkdir(parents=True,exist_ok=True);p.write_text(text);return p
fido=fragments.joinpath('16702.java.fragment').read_text();message=fragments.joinpath('14395.java.fragment').read_text()
prior=pathlib.Path('/tmp/opencode/samsung-pass-100-wave31/java25/sources/com/samsung/android/authfw/sdk/fido2/api/a$type.java').read_text()
# Preserve all genuine original constructor/field source. Insert new exact method
# fragment only; old unavailable comment is retained as historical fixture text.
write('com.samsung.android.authfw.sdk.fido2.api.a$type',prior.rsplit('}',1)[0]+fido+'\n}\n')
write('com.samsung.android.authfw.domain.common.shared.message.MessageUtil','package com.samsung.android.authfw.domain.common.shared.message; public final class MessageUtil {\n'+message+'\n}')
write('q8.j$type','package q8; public abstract class j$type extends java.io.IOException { public j$type(String s){super(s);} }')
write('y8.l$type','package y8; public class l$type extends q8.j$type { public l$type(String s){super(s);} }')
write('com.samsung.android.authfw.domain.fido2.shared.dictionary.PublicKeyCredentialCreationOptions','package com.samsung.android.authfw.domain.fido2.shared.dictionary; public class PublicKeyCredentialCreationOptions {}')
write('com.samsung.android.authfw.domain.common.shared.message.Message','package com.samsung.android.authfw.domain.common.shared.message; public class Message {}')
write('boundary.Trace','''package boundary; public class Trace { public static Throwable problem; public static String events=""; public static Object first,second; public static java.util.Optional result=java.util.Optional.of("ok"); public static java.util.Optional call(Object a,Object b) throws q8.j$type,y8.l$type { events+="T"; if(a!=first || b!=second)throw new AssertionError("arg identity"); Throwable e=problem; if(e instanceof y8.l$type)throw (y8.l$type)e; if(e instanceof q8.j$type)throw (q8.j$type)e; if(e instanceof RuntimeException)throw (RuntimeException)e; if(e instanceof Error)throw (Error)e; return result; } }''')
write('com.samsung.android.authfw.sdk.fido2.api.Fido2ApiClient','''package com.samsung.android.authfw.sdk.fido2.api; public class Fido2ApiClient { public static java.util.Optional d(Fido2ApiClient a,com.samsung.android.authfw.domain.fido2.shared.dictionary.PublicKeyCredentialCreationOptions b) throws q8.j$type,y8.l$type { return boundary.Trace.call(a,b); } }''')
write('com.samsung.android.authfw.domain.common.shared.json.JsonUtil','package com.samsung.android.authfw.domain.common.shared.json; public class JsonUtil { public static final JsonUtil$Companion Companion=new JsonUtil$Companion(); }')
write('com.samsung.android.authfw.domain.common.shared.json.JsonUtil$Companion','''package com.samsung.android.authfw.domain.common.shared.json; public class JsonUtil$Companion { public java.util.Optional toJson(Object value) throws q8.j$type,y8.l$type {return boundary.Trace.call(this,value);} }''')
write('kotlin.jvm.internal.n$type','''package kotlin.jvm.internal; public class n$type { public static void f(Object value,String name){boundary.Trace.events+="G";if(value==null)throw new NullPointerException(name);} }''')
write('Driver','''import boundary.Trace; public class Driver {
static int checks; static void require(boolean ok){checks++;if(!ok)throw new AssertionError("check "+checks);}
public static void main(String[] args){
var client=new com.samsung.android.authfw.sdk.fido2.api.Fido2ApiClient();var options=new com.samsung.android.authfw.domain.fido2.shared.dictionary.PublicKeyCredentialCreationOptions();
java.util.function.Supplier supplier=new com.samsung.android.authfw.sdk.fido2.api.a$type(client,options);
var message=new com.samsung.android.authfw.domain.common.shared.message.Message();
for(int mode=0;mode<2;mode++) {
 Trace.first=mode==0?client:com.samsung.android.authfw.domain.common.shared.json.JsonUtil.Companion;Trace.second=mode==0?options:message;Trace.events="";Trace.problem=null;
 Object result=mode==0?supplier.get():com.samsung.android.authfw.domain.common.shared.message.MessageUtil.toJson(message);require(result==Trace.result);require(Trace.events.equals(mode==0?"T":"GT"));
 for(Throwable e:new Throwable[]{new q8.j$type("checked"){},new y8.l$type("subclass"),new IllegalStateException("runtime"),new OutOfMemoryError("preconstructed")}) {
 e.addSuppressed(new Exception("suppressed"));StackTraceElement[] stack=e.getStackTrace();String text=e.getMessage();Trace.problem=e;Trace.events="";
 try{if(mode==0)supplier.get();else com.samsung.android.authfw.domain.common.shared.message.MessageUtil.toJson(message);throw new AssertionError("did not throw");}catch(Throwable observed){require(observed==e);require(java.util.Arrays.equals(stack,observed.getStackTrace()));require(observed.getMessage().equals(text));require(observed.getSuppressed().length==1);require(Trace.events.equals(mode==0?"T":"GT"));}
 }
}
Trace.events="";try{com.samsung.android.authfw.domain.common.shared.message.MessageUtil.toJson(null);throw new AssertionError();}catch(NullPointerException expected){require(Trace.events.equals("G"));require(expected.getMessage().equals("<this>"));}
System.out.println("PASS "+checks);
}}
''')
report={'kind':'unchanged original emitted method fragments plus explicitly fabricated dependency boundaries','fragment_dir':str(fragments),'no_whole_apk_claim':True,'original_constructor_retained':True,'root':str(out),'jdk':{}}
for release,root in [('17','/home/jtjisgod/tools/jdk-17.0.13+11'),('25','/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1')]:
 root=pathlib.Path(root);classes=out/('classes'+release);classes.mkdir();cmd=[str(root/'bin/javac'),'--release',release,'-d',str(classes)]+[str(p) for p in src.rglob('*.java')]
 build=subprocess.run(cmd,capture_output=True,text=True);(out/('javac'+release+'.log')).write_text(build.stdout+build.stderr)
 assert build.returncode==0,build.stderr
 run=subprocess.run([str(root/'bin/java'),'-cp',str(classes),'Driver'],capture_output=True,text=True);assert run.returncode==0,run.stderr
 dis=subprocess.run([str(root/'bin/javap'),'-p','-c','-s','-classpath',str(classes),'com.samsung.android.authfw.sdk.fido2.api.a$type'],capture_output=True,text=True);(out/('javap'+release+'.txt')).write_text(dis.stdout)
 helper=dis.stdout.split('private static <T extends java.lang.Throwable>')[1];assert 'descriptor: (Ljava/lang/Throwable;)Ljava/lang/RuntimeException;' in helper and 'athrow' in helper and 'checkcast' not in helper
 report['jdk'][release]={'compile_exit':build.returncode,'run_exit':run.returncode,'stdout':run.stdout.strip(),'javac_sha256':hashlib.sha256((root/'bin/javac').read_bytes()).hexdigest(),'erased_helper_no_checkcast':True}
report['fragments_sha256']={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in fragments.glob('*.fragment')}
(out/'report.json').write_text(json.dumps(report,indent=2));print(json.dumps(report,indent=2))
