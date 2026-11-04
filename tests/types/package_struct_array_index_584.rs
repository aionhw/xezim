//! IEEE 1800-2017 §7.4.1 / §26.3: a variable index of a package array of
//! packed structs selects an element (#584, reported by davekeeshan). Inside
//! a package function `ENC[fmt].exp_bits` was a one-bit select at runtime,
//! so `exp_of` returned 0 and `bias` 2^32-1 (fpnew's FP add returned +Inf).
//! Also an ascending `[0:N]` and a descending `[N:0]` array, a module-local
//! array, an ascending mask, a fused `assign v[i]` on `logic [0:N]`, and the
//! package-qualified field read `p::ENC[f].man_bits` from a class method,
//! which read 0.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

const PKG: &str = "package p;
  typedef struct packed { int unsigned exp_bits; int unsigned man_bits; } enc_t;
  localparam enc_t [0:4] ENC = '{ '{8, 23}, '{11, 52}, '{5, 10}, '{5, 2}, '{8, 7} };
  localparam enc_t [4:0] DEC = '{ '{8, 23}, '{11, 52}, '{5, 10}, '{5, 2}, '{8, 7} };
  localparam logic [0:4] MASK = 5'b10110;
  function automatic int unsigned exp_of(int unsigned fmt);
    return ENC[fmt].exp_bits;
  endfunction
  function automatic int unsigned bias(int unsigned fmt);
    return unsigned'(2**(ENC[fmt].exp_bits-1)-1);
  endfunction
endpackage
";

#[test]
fn package_function_indexes_an_array_of_structs() {
    let msgs = messages(&format!(
        "{PKG}module top;
  logic [2:0] fmt = 3'd0;
  initial #1 $display(\"bias=%0d exp=%0d\", p::bias(fmt), p::exp_of(fmt));
endmodule
"
    ));
    assert_line(&msgs, "bias=127 exp=8");
}

#[test]
fn variable_index_of_package_and_local_struct_arrays() {
    let msgs = messages(&format!(
        "{PKG}class C;
  function int unsigned q(int unsigned f); return p::ENC[f].man_bits; endfunction
  function int unsigned d(int unsigned f); return p::DEC[f].man_bits; endfunction
endclass
module top;
  import p::*;
  typedef struct packed {{ logic [3:0] a; logic [3:0] b; }} ab_t;
  localparam ab_t [0:2] LOC = '{{ '{{4'h1, 4'h2}}, '{{4'h3, 4'h4}}, '{{4'h5, 4'h6}} }};
  logic [2:0] f;
  logic [0:3] v;
  logic in_v;
  assign v[0] = in_v;
  assign v[3] = 1'b1;
  C c = new;
  initial begin
    in_v = 1;
    for (int i = 0; i < 5; i++) begin
      f = i;
      #1 $display(\"f=%0d enc=%0d/%0d dec=%0d/%0d mask=%b loc=%h q=%0d d=%0d\", f,
        ENC[f].exp_bits, ENC[f].man_bits, DEC[f].exp_bits, DEC[f].man_bits, MASK[f],
        (f < 3) ? LOC[f] : 8'h0, c.q(f), c.d(f));
    end
    #1 $display(\"v=%b\", v);
  end
endmodule
"
    ));
    for want in [
        "f=0 enc=8/23 dec=8/7 mask=1 loc=12 q=23 d=7",
        "f=1 enc=11/52 dec=5/2 mask=0 loc=34 q=52 d=2",
        "f=2 enc=5/10 dec=5/10 mask=1 loc=56 q=10 d=10",
        "f=3 enc=5/2 dec=11/52 mask=1 loc=00 q=2 d=52",
        "f=4 enc=8/7 dec=8/23 mask=0 loc=00 q=7 d=23",
        "v=1xx1",
    ] {
        assert_line(&msgs, want);
    }
}
