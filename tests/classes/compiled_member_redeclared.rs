//! §8.14 / §8.10: a member read through a handle names the copy declared at
//! or above the handle's STATIC type, even when the object's class
//! redeclares the member. Compiled method bodies must agree for a plain
//! handle, a formal, and an element of a class-handle collection. Each test
//! runs at tier 0 (a child process compiles every method on its first
//! call) and again at the default tier, where the 1500-iteration loop
//! crosses the compile threshold. Expected values come from the reference
//! simulator.

use xezim::simulate;

#[test]
fn compiled_member_reads_use_static_type_copy() {
    let _ = super::compiled_method_test_env::eager();
    let src = r#"
module top;
class node;
  int val;
  string tag;
  function new(); val = 1; tag = "base"; endfunction
endclass
class dnode extends node;
  int val;
  string tag;
  function new(); super.new(); val = 2; tag = "der"; endfunction
endclass
class box;
  node m[string];
  node q[$];
  dnode dq[$];
  node hm;
  function int rv(string k); return m[k].val; endfunction
  function string rt(int i); return q[i].tag; endfunction
  function int rd(int i); return dq[i].val + q[i].val * 10; endfunction
  function int r1(); return hm.val; endfunction
  function int r2(node h); return h.val; endfunction
endclass
initial begin
  automatic box b = new;
  automatic dnode d = new;
  automatic int a = 0, c = 0, e = 0, f = 0;
  automatic string s;
  b.m["x"] = d; b.q.push_back(d); b.dq.push_back(d); b.hm = d;
  for (int i = 0; i < 1500; i++) begin
    a += b.rv("x"); s = b.rt(0); c += b.rd(0); e += b.r1(); f += b.r2(d);
  end
  $display("T|a=%0d s=%s c=%0d e=%0d f=%0d", a, s, c, e, f);
end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let out: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(out, ["T|a=1500 s=base c=18000 e=1500 f=1500"]);
}

/// §8.14: a base-class method's BARE read and write of a string member a
/// derived class redeclares use the base's copy. Compiled, the bare
/// spelling stored to and loaded from the derived class's copy.
#[test]
fn compiled_bare_string_member_uses_declaring_class_copy() {
    let _ = super::compiled_method_test_env::eager();
    let src = r#"
class node;
  string tag;
  function void set(string t); tag = t; endfunction
  function string get(); return tag; endfunction
endclass
class dnode extends node;
  string tag;
  function void dset(string t); tag = t; endfunction
endclass
module top;
  initial begin
    automatic dnode d = new;
    automatic node n = d;
    d.dset("derived");
    n.set("base");
    $display("T|base=%s derived=%s get=%s", n.tag, d.tag, n.get());
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let out: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(out, ["T|base=base derived=derived get=base"]);
}
