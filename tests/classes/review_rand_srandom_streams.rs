//! §18.5.4, §18.13, §18.14 (#1528 follow-up): after `srandom()` or
//! `set_randstate()` on an object, its `randomize()` repeats the values it
//! drew after the same seed or state. That holds for a `dist` constraint,
//! whose item schedule is dealt from the object's stream and starts over with
//! it, and for a nested object randomized as part of the outer object, which
//! draws from the outer object's stream when it has none of its own. The
//! reference simulator repeats every form here.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .unwrap()
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .collect()
}

#[test]
fn reseeding_repeats_dist_and_nested_object_draws() {
    let out = lines(
        r#"
class c_dist;
  rand bit [7:0] v;
  constraint k { v dist { [0:9] := 1, [10:255] :/ 1 }; }
endclass
class c_inner;
  rand bit [7:0] x;
endclass
class c_nest;
  rand c_inner in;
  rand bit [7:0] y;
  function new; in = new; endfunction
endclass
module t;
  c_dist d = new, d2 = new;
  c_nest n = new, n2 = new;
  int a[3][6], b[3][6], junk;
  initial begin
    for (int r = 0; r < 3; r++) begin
      d.srandom(5);
      n.srandom(6);
      for (int i = 0; i < 6; i++) begin
        void'(d.randomize());
        void'(n.randomize());
        a[r][i] = d.v;
        b[r][i] = {n.in.x, n.y};
      end
      junk = $urandom;
      void'(d2.randomize());
      void'(n2.randomize());
    end
    $display("dist %0d", a[0] == a[1] && a[1] == a[2]);
    $display("nested %0d", b[0] == b[1] && b[1] == b[2]);
  end
endmodule
"#,
    );
    assert_eq!(out, ["dist 1", "nested 1"]);
}

#[test]
fn set_randstate_repeats_dist_draws() {
    let out = lines(
        r#"
class C;
  rand bit [7:0] v;
  rand bit [3:0] w;
  constraint k { v dist { [0:9] := 1, [10:255] :/ 1 }; w < 12; }
endclass
module t;
  C c = new, o = new;
  string st;
  int a[6], b[6], junk;
  initial begin
    void'(c.randomize());
    st = c.get_randstate();
    for (int i = 0; i < 6; i++) begin void'(c.randomize()); a[i] = {c.v, c.w}; end
    junk = $urandom;
    void'(o.randomize());
    c.set_randstate(st);
    for (int i = 0; i < 6; i++) begin void'(c.randomize()); b[i] = {c.v, c.w}; end
    $display("randstate %0d", a == b);
  end
endmodule
"#,
    );
    assert_eq!(out, ["randstate 1"]);
}
