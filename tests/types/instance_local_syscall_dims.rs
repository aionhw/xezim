//! §6.20.2, §20.8.1, §23.10: a statement-local declaration in an
//! instantiated module whose packed dimension calls a value system function on
//! the instance's parameters (`logic [$clog2(P)-1:0] v;`) takes this
//! instance's value. The local used to keep its bare `P`, which the
//! design-wide parameter map holds only as `u.P`, and came out one bit wide in
//! a process and 32 bits wide in a function, while `[P-1:0]` was right. A
//! keccak `theta()` declaring `logic [$clog2(W)-1:0] index_z;` inside its loop
//! computed every SHA3 digest wrong.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn instance_local_syscall_dims_take_the_instance_parameters() {
    let src = r#"
module m #(parameter int P = 16, localparam int Q = P / 4) ();
  function automatic int f_loop();
    int r;
    for (int i = 0; i < 1; i++) begin
      logic [$clog2(P)-1:0] v;
      v = '1;
      r = v;
    end
    return r;
  endfunction
  function automatic int f_top();
    logic [$clog2(Q)-1:0] v;
    v = '1;
    return v;
  endfunction
  task automatic t_width(output int w);
    logic [$signed(P)-1:0] v;
    w = $bits(v);
  endtask
  initial begin
    logic [$clog2(P)-1:0]  a;
    logic [$signed(P)-1:0] b;
    logic [$clog2(Q)-1:0]  c;
    logic [int'(P)-1:0]    d;
    int tw;
    a = '1;
    t_width(tw);
    #1 $display("P=%0d a=%0d/%0d b=%0d c=%0d d=%0d fl=%0d ft=%0d tw=%0d",
                P, $bits(a), a, $bits(b), $bits(c), $bits(d), f_loop(), f_top(), tw);
  end
endmodule
module top;
  m #(.P(256)) u256 ();
  m #(.P(64))  u64 ();
endmodule
"#;
    let out = lines(src);
    for want in [
        "P=256 a=8/255 b=256 c=6 d=256 fl=255 ft=63 tw=256",
        "P=64 a=6/63 b=64 c=4 d=64 fl=63 ft=15 tw=64",
    ] {
        assert!(
            out.iter().any(|m| m == want),
            "missing {want:?}; got {out:?}"
        );
    }
}

/// §20.6.2, §11.7: `$bits` of an instance parameter, `$signed` of an
/// 8-bit `8'hff` (-1, not 255), `$unsigned`, a cast and a nested `$clog2`
/// in the locals of a function and a loop block, per instance; and the
/// locals of an interface function and a class method of the instance.
/// Expected values are the reference simulator's.
#[test]
fn sized_system_functions_and_other_scopes_of_the_instance() {
    let out = lines(
        r#"
interface ifc #(parameter int P = 4);
  function automatic int f();
    logic [$clog2(P)-1:0] a;
    return $bits(a);
  endfunction
endinterface
module m #(parameter int P = 4, parameter logic [7:0] Q = 8'hff);
  ifc #(.P(P*2)) i();
  class C;
    function int g();
      logic [$clog2(P)-1:0] a;
      return $bits(a);
    endfunction
  endclass
  function automatic int f();
    logic [$clog2(P)-1:0] a;
    logic [int'(P)-1:0] c;
    logic [$unsigned(P)-1:0] d;
    logic [($signed(Q) < 0 ? 3 : 5)-1:0] e;
    logic [$clog2(P*2)-1:0] g;
    return $bits(a) * 1000000 + $bits(c) * 1000 + $bits(d) + $bits(e) * 100000000 + $bits(g)*10000;
  endfunction
  initial begin
    C c = new;
    #1;
    for (int i = 0; i < 1; i++) begin
      logic [$clog2(P)-1:0] a;
      logic [$signed(P)-1:0] b;
      logic [$bits(P)-1:0] h;
      bit [$clog2(P):0] k [$clog2(P)];
      $display("P=%0d proc a=%0d b=%0d h=%0d k=%0d/%0d f=%0d", P, $bits(a), $bits(b), $bits(h), $bits(k[0]), $size(k), f());
    end
    $display("P=%0d ifc f=%0d class g=%0d", P, i.f(), c.g());
  end
endmodule
module top;
  m #(.P(256)) u256();
  m #(.P(16), .Q(8'h7f)) u16();
endmodule
"#,
    );
    for want in [
        "P=256 proc a=8 b=256 h=32 k=9/8 f=308346256",
        "P=16 proc a=4 b=16 h=32 k=5/4 f=504066016",
        "P=256 ifc f=9 class g=8",
        "P=16 ifc f=5 class g=4",
    ] {
        assert!(
            out.iter().any(|m| m == want),
            "missing {want:?}; got {out:?}"
        );
    }
}
