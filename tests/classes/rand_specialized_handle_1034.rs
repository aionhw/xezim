//! §18.4, §8.25: binding a rand handle's type parameter to a specialized
//! class preserves the handle and solves the pointed-to object's constraints.

#[test]
fn specialized_rand_handles_keep_identity_and_concrete_constraints() {
    let src = r#"
class P #(int N=1);
  rand bit [7:0] v;
  constraint k { v == N; }
endclass
class O #(type T=P);
  rand T h;
  function new(); h=new; endfunction
endclass
typedef P#(13) alias_t;
class D extends O#(P#(9)); endclass
class Outer;
  rand O#(P#(11)) child;
  function new(); child=new; endfunction
endclass
class Nullable;
  rand P#(3) h;
endclass
module top;
  initial begin
    O#(P#(7)) a=new;
    O#(.T(P#(.N(127)))) b=new;
    O#(alias_t) c=new;
    D d=new;
    Outer e=new;
    Nullable f=new;
    P#(7) pa=a.h;
    P#(127) pb=b.h;
    alias_t pc=c.h;
    P#(9) pd=d.h;
    P#(11) pe=e.child.h;
    int ok;
    ok=a.randomize();
    if (!ok || a.h!=pa || a.h.v!=7) $fatal(1,"plain specialization");
    ok=b.randomize();
    if (!ok || b.h!=pb || b.h.v!=127) $fatal(1,"named specialization");
    ok=c.randomize();
    if (!ok || c.h!=pc || c.h.v!=13) $fatal(1,"typedef specialization");
    ok=d.randomize();
    if (!ok || d.h!=pd || d.h.v!=9) $fatal(1,"inherited specialization");
    ok=e.randomize();
    if (!ok || e.child.h!=pe || e.child.h.v!=11) $fatal(1,"nested specialization");
    ok=f.randomize();
    if (!ok || f.h!=null) $fatal(1,"null handle changed");
    a.h.v=8;
    ok=a.randomize() with {h.v==9;};
    if (ok || a.h!=pa || a.h.v!=8) $fatal(1,"failed solve changed object");
    $display("T|PASS");
  end
endmodule
"#;
    let sim = xezim::simulate(src, 100).expect("simulate");
    assert!(
        sim.output.iter().any(|x| x.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}
