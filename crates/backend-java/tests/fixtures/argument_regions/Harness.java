public final class Harness {
 static Big call(boolean original){return original?Original.make():Emitted.make();}
 public static void main(String[] args){boolean original=args[0].equals("original");String mode=args[1];Probe.mode=mode;
  if(mode.equals("oom")){
   call(original);Probe.trace.setLength(0);Probe.calls=0;Probe.constructions=0;
   Probe.retained=new byte[30000][];int n=0;try{while(n<Probe.retained.length)Probe.retained[n++]=new byte[4096];}catch(OutOfMemoryError expected){}
   boolean failed=false;try{call(original);}catch(OutOfMemoryError expected){failed=true;}
   int effects=Probe.calls+Probe.constructions+Probe.trace.length();Probe.retained=null;System.gc();if(!failed||effects!=0)throw new AssertionError("allocation must fail before argument effects");System.out.println("OOM:no-arguments");return;
  }
  Throwable failure=null;Big result=null;try{result=call(original);}catch(Throwable t){failure=t;}
  if(mode.equals("class")){if(failure!=Probe.classFailure||Probe.calls!=0||Probe.constructions!=0)throw new AssertionError("class-init");}
  else if(mode.equals("alternate")||mode.equals("F")||mode.equals("N1")||mode.equals("N2")||mode.equals("E")||mode.equals("T")){if(failure!=Probe.failure)throw new AssertionError("exception identity "+failure);}
  else{if(failure!=null||result.first!=Probe.token||result.second!=(mode.equals("null")?null:Probe.other))throw new AssertionError("value identity");}
  System.out.println(Probe.trace+":"+Probe.calls+":"+Probe.constructions+":"+(failure==null?"ok":"exact"));
 }
}
