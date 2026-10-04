public final class Harness {
 static Big call(boolean original){return original?Original.make():Emitted.make();}
 public static void main(String[] args){boolean original=args[0].equals("original");String mode=args[1];Probe.mode=mode;
  if(mode.equals("oom")||mode.equals("inneroom")){
   call(original);Probe.trace.setLength(0);Probe.calls=0;Probe.constructions=0;Probe.touches=0;if(mode.equals("inneroom"))Probe.reserve=new byte[150000];
   System.gc();Probe.retained=new byte[30000][];int n=0;try{while(n<Probe.retained.length)Probe.retained[n++]=new byte[4096];}catch(OutOfMemoryError expected){}
   if(mode.equals("inneroom")){Probe.reserve=null;System.gc();}
   boolean failed=false;try{call(original);}catch(OutOfMemoryError expected){failed=true;}
   int effects=Probe.calls+Probe.constructions+Probe.trace.length();Probe.retained=null;System.gc();if(!failed||(mode.equals("inneroom")?!Probe.trace.toString().equals("F")||Probe.calls!=0||Probe.constructions!=0:effects!=0))throw new AssertionError("allocation must fail before argument effects: failed="+failed+", trace="+Probe.trace+", calls="+Probe.calls+", ctors="+Probe.constructions);System.out.println(mode.equals("inneroom")?"OOM:after-prefix-before-inner-constructor":"OOM:no-arguments");return;
  }
  Throwable failure=null;Big result=null;try{result=call(original);}catch(Throwable t){failure=t;}
  if(mode.equals("class")||mode.equals("innerclass")){if(failure!=Probe.classFailure||Probe.calls!=0||Probe.constructions!=0)throw new AssertionError("class-init");}
  else if(mode.equals("alternate")||mode.equals("F")||mode.equals("N1")||mode.equals("N2")||mode.equals("E")||mode.equals("T")||mode.equals("I")||mode.equals("U1")||mode.equals("U2")){if(failure!=Probe.failure)throw new AssertionError("exception identity "+failure);}
  else{if(failure!=null||result.first!=Probe.token||result.second!=(mode.equals("null")?null:Probe.other))throw new AssertionError("value identity");}
  System.out.println(Probe.trace+":"+Probe.calls+":"+Probe.constructions+":"+(failure==null?"ok":"exact"));
 }
}
