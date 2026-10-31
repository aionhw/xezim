//! §11.6.1 / §11.8.2: an equality or relational comparison sizes both
//! operands to the wider of their own widths. In a compiled method a bare
//! instance member, or a member reached through a handle, reported width 0,
//! so with members on both sides the context collapsed and a shift operand
//! was evaluated in too few bits: `(burst << 3) != line` was 1 for 64 vs 64,
//! and an aligned 64-bit address read as misaligned. Expected values are the
//! reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

const SHAPES: &str = r#"
class cfg_t; int line = 64; endclass
class item;
  cfg_t cfg;
  bit [63:0] addr = 64'h1_0000_0000;
  int burst = 8;
  int line2 = 64;
  function int f1(); return (burst << 3) != cfg.line; endfunction
  function int f2(); return (burst << 3) != 64; endfunction
  function int f3(); int x = 64; return x != cfg.line; endfunction
  function int f4(); return (burst * 8) != cfg.line; endfunction
  function int f5(); return (burst << 3) != line2; endfunction
  function int f6(); return addr != ((addr >> 3) << 3); endfunction
  function int f7(); bit [63:0] a = addr; return a != ((a >> 3) << 3); endfunction
  function int f8(); return (addr >> 3) != 64'h2000_0000; endfunction
  function int f9(); int t = 3; return (burst << t) == cfg.line; endfunction
  function int f10(); return (cfg.line >> 3) == burst; endfunction
endclass
module top;
  initial begin
    automatic item x = new;
    x.cfg = new;
    $display("T|%0d %0d %0d %0d %0d %0d %0d %0d %0d %0d", x.f1(), x.f2(), x.f3(), x.f4(), x.f5(), x.f6(), x.f7(), x.f8(), x.f9(), x.f10());
  end
endmodule
"#;

const VALIDITY_CHECK: &str = r#"
class cfg_t;
  int data_width = 64;
  int line = 64;
  function int log_base_2(int v); int r = 0; while (v > 1) begin v = v >> 1; r++; end return r; endfunction
endclass
class item;
  cfg_t cfg;
  bit [63:0] addr = 64'h1_0000_0000;
  int burst = 8;
  function bit check(bit silent = 1);
    int ok = 1;
    int t = (cfg.log_base_2(cfg.data_width/8));
    if (addr != ((addr >> t) << t)) begin
      if (!silent) $display("T|misaligned t=%0d", t);
      ok = 0;
    end
    if ((burst << t) != cfg.line) begin
      if (!silent) $display("T|size %0d != %0d", burst << t, cfg.line);
      ok = 0;
    end
    return ok;
  endfunction
endclass
module top;
  initial begin
    automatic item x = new;
    x.cfg = new;
    $display("T|check=%0d", x.check(0));
    $display("T|check=%0d", x.check(0));
  end
endmodule
"#;

#[test]
fn member_operands_size_compiled_comparisons() {
    let _ = super::compiled_method_test_env::policies(&[("1", "0"), ("0", "0")]);
    assert_eq!(t_lines(SHAPES), ["T|0 0 0 0 0 0 0 0 1 1"]);
    assert_eq!(t_lines(VALIDITY_CHECK), ["T|check=1", "T|check=1"]);
}
