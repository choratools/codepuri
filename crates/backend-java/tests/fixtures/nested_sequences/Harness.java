public final class Harness {
 static Big call(boolean original){return original?Original.make():Emitted.make();}
 public static void main(String[] args){boolean original=args[0].equals("original");String mode=args[1];Probe.mode=mode;
  boolean innerOOM=mode.equals("innerOOM");
  if(mode.endsWith("OOM")){
   call(original);System.gc();Probe.trace.setLength(0);Probe.retained=new byte[30000][];int n=0;try{while(n<Probe.retained.length)Probe.retained[n++]=new byte[4096];}catch(OutOfMemoryError expected){}
   if(innerOOM){int first=n>45?n-45:0;for(int x=n-1;x>=first;x--)Probe.retained[x]=null;System.gc();}
   boolean failed=false;try{call(original);}catch(OutOfMemoryError expected){failed=true;}
   int effects=Probe.trace.length();Probe.retained=null;System.gc();if(!failed||effects!=0)throw new AssertionError("real NEW OOM must precede argument/constructor effects");System.out.println(mode+":no-effects");return;
  }
  Throwable failure=null;Big result=null;try{result=call(original);}catch(Throwable t){failure=t;}
  if(mode.equals("normal")){if(failure!=null||result.first!=Probe.first||result.third!=Probe.third||!result.second.equals("late"))throw new AssertionError("identity");}
  else if(failure!=(mode.endsWith("C")?Probe.initFailure:Probe.failure))throw new AssertionError("exact exception "+failure);
  System.out.println(Probe.trace+":"+(failure==null?"ok":"exact"));
 }
}
