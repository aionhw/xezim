//! §11.4.5 / §18.5: a one-bit `&` or `|` in a constraint is solved like
//! `&&` or `||` by every pass of the solve, not only by the CSP. Each line
//! below returned 0 with the members untouched: the operand was a one-bit
//! `rand bit` or a bit select (`data[0]`), which the CSP did not take for one
//! bit, or the other operand was a relation only the forced pass inverts
//! (`(data ^ 'hf) == 0`), which never saw the bitwise form. A `&` whose
//! operand is wider than one bit (`m`, the packed row `pk[1]`, the array
//! element `q[0]`) keeps its bitwise meaning: only bit 0 of that operand is
//! constrained. `std::randomize() with` takes the same rewrite.

use xezim::simulate;

const SRC: &str = r#"
typedef enum logic [2:0] {S_INSTR = 3'd1, S_DATA = 3'd2} s_e;
class item;
  rand s_e s;
  rand bit b;
  rand logic [31:0] data;
  rand logic [3:0] m;
  rand logic [1:0][3:0] pk;
  rand int unsigned q [2];
endclass
class declared;
  rand bit b;
  rand logic [31:0] data;
  constraint c { b & data == 32'h77; }
endclass
class bit_select;
  rand logic [31:0] data;
  constraint c { data[0] & data == 32'h31; }
endclass
class implied;
  rand s_e s;
  rand logic [31:0] data;
  constraint c { (s == S_DATA) -> (data[0] & data == 32'h31); }
endclass
module top;
  item it;
  declared d;
  bit_select bs;
  implied im;
  int ok, bad;
  logic [31:0] sd;
  bit sb;
  initial begin
    it = new; d = new; bs = new; im = new;
    ok = it.randomize() with { b & data == 32'h1234; };
    $display("bit: ok=%0d b=%0d data=%h", ok, it.b, it.data);
    ok = it.randomize() with { s == S_DATA & (data ^ 32'hf) == 32'h0; };
    $display("xor: ok=%0d s=%0d data=%h", ok, it.s, it.data);
    ok = d.randomize();
    $display("declared: ok=%0d b=%0d data=%h", ok, d.b, d.data);
    ok = bs.randomize();
    $display("select: ok=%0d data=%h", ok, bs.data);
    ok = im.randomize() with { s == S_DATA; };
    $display("implied: ok=%0d data=%h", ok, im.data);
    bad = 0;
    for (int i = 0; i < 8; i++) begin
      ok = it.randomize() with { m & (data[3:0] == 4'h5); };
      if (!ok || !(it.m[0] && it.data[3:0] == 5)) bad++;
      ok = it.randomize() with { pk[1] & (data[3:0] == 4'h6); };
      if (!ok || !(it.pk[1][0] && it.data[3:0] == 6)) bad++;
      ok = it.randomize() with { q[0] & (data[3:0] == 4'h7); };
      if (!ok || !(it.q[0][0] && it.data[3:0] == 7)) bad++;
    end
    $display("wide: bad=%0d", bad);
    ok = std::randomize(sb, sd) with { sb & sd == 32'h1234; };
    $display("std bit: ok=%0d b=%0d data=%h", ok, sb, sd);
    ok = std::randomize(sd) with { sd[0] & sd == 32'h31; };
    $display("std select: ok=%0d data=%h", ok, sd);
  end
endmodule
"#;

#[test]
fn one_bit_bitwise_operators_solve_in_every_pass() {
    let out: Vec<String> = simulate(SRC, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(
        out,
        vec![
            "bit: ok=1 b=1 data=00001234",
            "xor: ok=1 s=2 data=0000000f",
            "declared: ok=1 b=1 data=00000077",
            "select: ok=1 data=00000031",
            "implied: ok=1 data=00000031",
            "wide: bad=0",
            "std bit: ok=1 b=1 data=00001234",
            "std select: ok=1 data=00000031",
        ],
        "{out:?}"
    );
}
