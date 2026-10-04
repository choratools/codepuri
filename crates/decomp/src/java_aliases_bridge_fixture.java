interface Wire { Builder create(); }
class Builder { final String value; Builder(String v){value=v;} }
class TypedBuilder extends Builder { TypedBuilder(String v){super(v);} }
class Parent implements Wire {
 final TypedBuilder token=new TypedBuilder("typed");
 public final TypedBuilder create$typed(){DispatchBoundary.events+="T";return token;}
 public Builder create(){return create$typed();}
}
class Child extends Parent { public Builder create(){return super.create$typed();} }
class GrandChild extends Child { final Builder other=new Builder("virtual");public Builder create(){DispatchBoundary.events+="G";return other;} }
public class DispatchBoundary {
 static String events="";
 public static void main(String[]a)throws Exception {
  for(Parent p:new Parent[]{new Parent(),new Child(),new GrandChild()}){
   events="";Builder wide=((Wire)p).create();TypedBuilder narrow=p.create$typed();
   boolean same=wide==narrow;System.out.println(p.getClass().getSimpleName()+":"+wide.value+":"+narrow.value+":"+same+":"+events);
  }
 }
}
