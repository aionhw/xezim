//! §18.5.12, §11.8.2: an equality that reaches its random variable through
//! `^`, `~^`, `~`, `+` or `-` has exactly one solution at the comparison's
//! context width, and both solvers find it. `std::randomize` returned 0 for
//! `(v ^ 32'hf) == 0` on a 32-bit variable: only interval narrowing saw the
//! equality, and the joint search gave up. Class constraints solved
//! `data ^ 'hf` only because three probes of it happen to lie on a line, so
//! `data ^ 'hf0` and `h ^ 16'h8001` failed. The solution must survive the
//! extension to the context width: an 8-bit `p` cannot make bit 8 of
//! `p ^ 32'h100` clear, and those calls still fail. Expected values come
//! from the reference simulator.

use xezim::simulate;

const SRC: &str = r#"
class C;
  rand bit [31:0] data;
  rand bit [15:0] h;
  rand byte sb;
  rand int si;
  rand bit [7:0] p;
endclass
class D;
  rand bit [31:0] data;
  constraint c { (data ^ 32'hdead) == 32'hbeef; }
endclass
class E;
  rand bit [7:0] p;
  constraint c { (p ^ 32'h100) == 32'h5; }
endclass
module top;
  C c; D d; E e;
  int ok;
  logic [31:0] l32; bit [31:0] b32; bit [15:0] b16; bit [7:0] p; byte sb; int si;
  initial begin
    c = new; d = new; e = new;
    ok = std::randomize(l32) with { (l32 ^ 32'hf) == 32'h0; };
    $display("T|std l32: ok=%0d v=%h", ok, l32);
    ok = std::randomize(b32) with { (b32 ^ 32'hdeadbeef) == 32'h12345678; };
    $display("T|std b32: ok=%0d v=%h", ok, b32);
    ok = std::randomize(b16) with { (b16 ^ 16'hf) == 16'h0; };
    $display("T|std b16: ok=%0d v=%h", ok, b16);
    ok = std::randomize(b32) with { 32'h3 == (b32 ^ 32'hf0); };
    $display("T|std rev: ok=%0d v=%h", ok, b32);
    ok = std::randomize(b32) with { ~b32 == 32'hfffffff0; };
    $display("T|std not: ok=%0d v=%h", ok, b32);
    ok = std::randomize(b32) with { (b32 ~^ 32'h0f0f0f0f) == 32'hffffffff; };
    $display("T|std xnor: ok=%0d v=%h", ok, b32);
    ok = std::randomize(b32) with { ((b32 ^ 32'h3) - 32'h1) == 32'h10; };
    $display("T|std chain: ok=%0d v=%h", ok, b32);
    ok = std::randomize(b32) with { -(~b32) == 32'h8; };
    $display("T|std negnot: ok=%0d v=%h", ok, b32);
    ok = std::randomize(p) with { (p ^ 32'h100) == 32'h105; };
    $display("T|std ext: ok=%0d v=%h", ok, p);
    ok = std::randomize(sb) with { (sb + 16'sd1) == -16'sd127; };
    $display("T|std signed: ok=%0d v=%0d", ok, sb);
    ok = c.randomize() with { (data ^ 32'hf0) == 32'h3; };
    $display("T|cls xor: ok=%0d v=%h", ok, c.data);
    ok = c.randomize() with { (32'h5a5a ^ data) == 32'h3; };
    $display("T|cls xor left: ok=%0d v=%h", ok, c.data);
    ok = c.randomize() with { (data ~^ 32'hf0f0f0f0) == 32'h0; };
    $display("T|cls xnor: ok=%0d v=%h", ok, c.data);
    ok = c.randomize() with { (h ^ 16'h8001) == 16'h0; };
    $display("T|cls h: ok=%0d v=%h", ok, c.h);
    ok = c.randomize() with { (sb ^ 8'sh0f) == -8'sd1; };
    $display("T|cls signed: ok=%0d v=%0d", ok, c.sb);
    ok = c.randomize() with { ~p == 32'hfffffff0; };
    $display("T|cls not ext: ok=%0d v=%h", ok, c.p);
    ok = d.randomize();
    $display("T|cls declared: ok=%0d v=%h", ok, d.data);
    // No solution: the 8-bit `p` is zero-extended, so bit 8 stays set.
    ok = e.randomize();
    $display("T|cls none: ok=%0d", ok);
    ok = std::randomize(p) with { (p ^ 32'h100) == 32'h5; };
    $display("T|std none: ok=%0d", ok);
    ok = std::randomize(sb) with { (sb + 16'sd1) == -16'sd128; };
    $display("T|std signed none: ok=%0d", ok);
    ok = std::randomize(b32) with { (b32 ^ 32'h1) == 32'h4; (b32 ^ 32'h2) == 32'h4; };
    $display("T|std conflict: ok=%0d", ok);
  end
endmodule
"#;

#[test]
fn bijective_equalities_solve_exactly() {
    let out: Vec<String> = simulate(SRC, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        out,
        vec![
            "T|std l32: ok=1 v=0000000f",
            "T|std b32: ok=1 v=cc99e897",
            "T|std b16: ok=1 v=000f",
            "T|std rev: ok=1 v=000000f3",
            "T|std not: ok=1 v=0000000f",
            "T|std xnor: ok=1 v=0f0f0f0f",
            "T|std chain: ok=1 v=00000012",
            "T|std negnot: ok=1 v=00000007",
            "T|std ext: ok=1 v=05",
            "T|std signed: ok=1 v=-128",
            "T|cls xor: ok=1 v=000000f3",
            "T|cls xor left: ok=1 v=00005a59",
            "T|cls xnor: ok=1 v=0f0f0f0f",
            "T|cls h: ok=1 v=8001",
            "T|cls signed: ok=1 v=-16",
            "T|cls not ext: ok=1 v=0f",
            "T|cls declared: ok=1 v=00006042",
            "T|cls none: ok=0",
            "T|std none: ok=0",
            "T|std signed none: ok=0",
            "T|std conflict: ok=0",
        ],
        "{out:?}"
    );
}
