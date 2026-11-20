//! IEEE 1800-2023 §11.8.2 / §6.12: an integral operand of a REAL operator is
//! converted to real from its own SELF-determined value. A real operand used
//! to lend its 64-bit width to the integral side, so with `int unsigned u =
//! 1`, `(u - 2) * 1.0` converted a 64-bit all-ones (18446744073709551616.0)
//! instead of the 32-bit 4294967295 (4294967295.0, as the reference
//! simulator gives). The same held for `r = u - 2` into a real variable,
//! whose 64 bits are not a context width either, and for the other operand
//! of a comparison or a `?:` with a real arm.
//!
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let mut v: Vec<String> = simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    v.sort();
    v
}

fn sorted(lines: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

/// Integral operands of every width and signedness reaching a real operator at run time (through function arguments): x bits, `$itor`, real assignment targets, comparisons, `?:` with a real arm, real parameters and variables on the other side.
#[test]
fn run_time_integral_operands() {
    let got = t_lines(
        r#"
module top;
  function automatic void f3(int unsigned u, int s, shortint unsigned su, shortint ss, byte sb, byte unsigned ub,
                             longint unsigned lu, longint ls, logic [7:0] lx, logic [31:0] l32, logic signed [7:0] lsx,
                             bit [15:0] b16, logic [3:0] l4);
    real r; shortreal sr;
      r = (u - 2) * 1.0; $display("T|U1 %f", r);
      r = (u - 2) + 0.5; $display("T|U2 %f", r);
      r = u - 2 + 0.5; $display("T|U3 %f", r);
      r = u - 2; $display("T|U4 %f", r);
      r = $itor(u - 2); $display("T|U5 %f", r);
      $display("T|U6 %0d", (u - 2) > 1.0);
      $display("T|U7 %0d", (u - 2) * 1.0 < 0.0);
      $display("T|U8 %f", (u - 2) / 2.0);
      r = (s - 2) * 1.0; $display("T|S1 %f", r);
      r = s - 2 + 0.5; $display("T|S2 %f", r);
      $display("T|S3 %0d", (s - 2) < 0.0);
      r = (su - 2) * 1.0; $display("T|SU1 %f", r);
      r = (su - 16'd2) * 1.0; $display("T|SU2 %f", r);
      r = (ss - 16'sd2) * 1.0; $display("T|SS1 %f", r);
      r = (ub - 8'd2) * 1.0; $display("T|UB1 %f", r);
      r = (ub - 2) * 1.0; $display("T|UB2 %f", r);
      r = (sb - 8'sd2) * 1.0; $display("T|SB1 %f", r);
      sb = -128; r = sb * 1.0; $display("T|SB2 %f", r);
      r = (lu - 2) * 1.0; $display("T|LU1 %f", r);
      r = (ls - 2) * 1.0; $display("T|LS1 %f", r);
      r = lx * 1.0; $display("T|LX1 %f", r);
      r = lx; $display("T|LX2 %f", r);
      r = lx + 0.5; $display("T|LX3 %f", r);
      r = l32 * 1.0; $display("T|L32 %f", r);
      r = (l32 + 1) * 1.0; $display("T|L32b %f", r);
      r = lsx * 1.0; $display("T|LSX %f", r);
      r = (lsx - 8'sd1) * 0.5; $display("T|LSX2 %f", r);
      r = (b16 - 16'd5) * 1.0; $display("T|B16 %f", r);
      l4 = 4'd2; r = (l4 - 4'd3) * 1.0; $display("T|L4 %f", r);
      r = (l4 - 3) * 1.0; $display("T|L4b %f", r);
      sr = (u - 2) * 1.0; $display("T|SR %f", sr);
      r = (u - 2) * 1.0 + (s - 2) * 1.0; $display("T|MX %f", r);
      r = 1.0 * (u - 2); $display("T|MX2 %f", r);
      r = (u - 2) ** 1.0; $display("T|PW %f", r);
      r = (u * 2 - 3) * 1.0; $display("T|U9 %f", r);
      r = 2.0 - (u - 2); $display("T|U10 %f", r);
      r = (u == 1) ? (u - 2) : 1.5; $display("T|U11 %f", r);
      $display("T|U12 %0d", (u - 2) == 4294967295.0);
      $display("T|U13 %0d", (u - 2) >= 1.0e10);
      r = real'(u - 2); $display("T|U14 %f", r);
      r = (u - 2) * 1.0; $display("T|U15 %e", r);
      $display("T|U16 %f", $itor(su - 16'd2));
      $display("T|U17 %f", (ub - 8'd2) + 0.0);
  endfunction
  function automatic void f4(int unsigned u, int unsigned x, byte unsigned ub, logic [7:0] l8, shortint unsigned su,
                             longint unsigned lu, real r0);
    real r;
    r = r0;
      $display("T|C1 %0d", (u - 2) > 1.0);
      $display("T|C2 %0d", u - 2 > 1.0);
      $display("T|C3 %0d", (u - 32'd2) > 1.0);
      $display("T|C4 %0d", x > 1.0);
      $display("T|C5 %0d", x == 4294967295.0);
      $display("T|C6 %0d", (u - 2) > r);
      $display("T|C7 %0d", 1.0 < (u - 2));
      $display("T|C8 %0d", (ub - 8'd2) > 1.0);
      $display("T|C9 %0d", (l8 - 8'd2) > 1.0);
      $display("T|C10 %0d", (su - 16'd2) > 1.0);
      $display("T|C11 %0d", (u + 32'hFFFF_FFFE) > 1.0);
      $display("T|C12 %0d", x >= 1.0e9);
      $display("T|C13 %0d", (x + 0) > 1.0);
      $display("T|C14 %0d", (u - 2) != 4294967295.0);
      $display("T|C15 %0d", (u - 2) == -1.0);
      $display("T|C16 %0d", (lu - 2) > 1.0);
      $display("T|C17 %0d", (ub - 2) > 1.0);
      $display("T|C18 %0d", 32'hFFFF_FFFF > 1.0);
      $display("T|C19 %0d", (u - 2) < 0.0);
      r = (u == 1) ? (u - 2) : 1.5; $display("T|T1 %f", r);
      r = (u == 1) ? x : 1.5; $display("T|T2 %f", r);
      r = (u == 1) ? (ub - 8'd2) : 1.5; $display("T|T3 %f", r);
      r = (u == 1) ? (u - 2) : 1; $display("T|T4 %f", r);
      $display("T|T5 %f", (u == 1) ? (u - 2) : 1.5);
      r = (u == 0) ? 1.5 : (u - 2); $display("T|T6 %f", r);
      r = x; $display("T|A1 %f", r);
      r = (u - 2); $display("T|A2 %f", r);
      r = (ub - 8'd2); $display("T|A3 %f", r);
      r = (ub - 2); $display("T|A4 %f", r);
      r = (l8 - 8'd2); $display("T|A5 %f", r);
      r = x + 1; $display("T|A6 %f", r);
      r = x + 1.0; $display("T|A7 %f", r);
      r = -(u - 2) * 1.0; $display("T|A8 %f", r);
      r = ~u * 1.0; $display("T|A9 %f", r);
      r = (u << 31) * 2.0; $display("T|A10 %f", r);
      r = {u, u} * 1.0; $display("T|A11 %f", r);
      r = (u - 2) * 1.0; $display("T|A12 %f", r);
      r = 64'hFFFF_FFFF_FFFF_FFFF * 1.0; $display("T|A13 %f", r);
      r = 64'shFFFF_FFFF_FFFF_FFFF * 1.0; $display("T|A14 %f", r);
      r = (lu - 2) * 1.0; $display("T|A15 %e", r);
  endfunction
  function automatic void f5(int unsigned u, real r, real z, shortreal sr);
    localparam real RP = 1.0;
      $display("T|D1 %0d", (u - 2) > r);
      $display("T|D2 %0d", (u - 2) > RP);
      $display("T|D3 %0d", (u - 2) > PP);
      $display("T|D4 %0d", (u - 2) > (r + 0.0));
      $display("T|D5 %0d", (u - 2) > r * 1.0);
      $display("T|D6 %0d", (u - 2) < z);
      $display("T|D7 %0d", (u - 2) > sr);
      $display("T|D8 %0d", r < (u - 2));
      $display("T|D9 %0d", (u - 2) == -r);
      $display("T|D10 %0d", (u - 2) > 1);
      $display("T|D11 %0d", (u - 2) > $itor(1));
      $display("T|D12 %0d", (u - 2) > real'(1));
      r = (u == 1) ? (u - 2) : z; $display("T|E1 %f", r);
      r = (u == 1) ? (u - 2) : RP; $display("T|E2 %f", r);
      r = (u == 1) ? (u - 2) : 1.5; $display("T|E3 %f", r);
      r = (u - 2) * r; $display("T|E4 %f", r);
      r = (u - 2) + r; $display("T|E5 %f", r);
  endfunction
  parameter real PP = 1.0;
  initial begin
    f3(1, 1, 1, 1, 1, 1, 1, 1, 8'bxxxx_0011, 32'hFFFF_FFFF, -3, 3, 2);
    f4(1, 32'hFFFF_FFFF, 1, 1, 1, 1, 1.0);
    f5(1, 1.0, 0.0, 1.0);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        sorted(&[
            "T|U1 4294967295.000000",
            "T|U2 4294967295.500000",
            "T|U3 4294967295.500000",
            "T|U4 4294967295.000000",
            "T|U5 4294967295.000000",
            "T|U6 1",
            "T|U7 0",
            "T|U8 2147483647.500000",
            "T|S1 -1.000000",
            "T|S2 -0.500000",
            "T|S3 1",
            "T|SU1 4294967295.000000",
            "T|SU2 65535.000000",
            "T|SS1 -1.000000",
            "T|UB1 255.000000",
            "T|UB2 4294967295.000000",
            "T|SB1 -1.000000",
            "T|SB2 -128.000000",
            "T|LU1 18446744073709551616.000000",
            "T|LS1 -1.000000",
            "T|LX1 3.000000",
            "T|LX2 3.000000",
            "T|LX3 3.500000",
            "T|L32 4294967295.000000",
            "T|L32b 0.000000",
            "T|LSX -3.000000",
            "T|LSX2 -2.000000",
            "T|B16 65534.000000",
            "T|L4 15.000000",
            "T|L4b 4294967295.000000",
            "T|SR 4294967295.000000",
            "T|MX 4294967294.000000",
            "T|MX2 4294967295.000000",
            "T|PW 4294967295.000000",
            "T|U9 4294967295.000000",
            "T|U10 -4294967293.000000",
            "T|U11 4294967295.000000",
            "T|U12 1",
            "T|U13 0",
            "T|U14 4294967295.000000",
            "T|U15 4.294967e+09",
            "T|U16 65535.000000",
            "T|U17 255.000000",
            "T|C1 1",
            "T|C2 1",
            "T|C3 1",
            "T|C4 1",
            "T|C5 1",
            "T|C6 1",
            "T|C7 1",
            "T|C8 1",
            "T|C9 1",
            "T|C10 1",
            "T|C11 1",
            "T|C12 1",
            "T|C13 1",
            "T|C14 0",
            "T|C15 0",
            "T|C16 1",
            "T|C17 1",
            "T|C18 1",
            "T|C19 0",
            "T|T1 4294967295.000000",
            "T|T2 4294967295.000000",
            "T|T3 255.000000",
            "T|T4 4294967295.000000",
            "T|T5 4294967295.000000",
            "T|T6 4294967295.000000",
            "T|A1 4294967295.000000",
            "T|A2 4294967295.000000",
            "T|A3 255.000000",
            "T|A4 4294967295.000000",
            "T|A5 255.000000",
            "T|A6 0.000000",
            "T|A7 4294967296.000000",
            "T|A8 1.000000",
            "T|A9 4294967294.000000",
            "T|A10 4294967296.000000",
            "T|A11 4294967297.000000",
            "T|A12 4294967295.000000",
            "T|A13 18446744073709551616.000000",
            "T|A14 -1.000000",
            "T|A15 1.844674e+19",
            "T|D1 1",
            "T|D2 1",
            "T|D3 1",
            "T|D4 1",
            "T|D5 1",
            "T|D6 0",
            "T|D7 1",
            "T|D8 1",
            "T|D9 0",
            "T|D10 1",
            "T|D11 1",
            "T|D12 1",
            "T|E1 4294967295.000000",
            "T|E2 4294967295.000000",
            "T|E3 4294967295.000000",
            "T|E4 18446744065119617024.000000",
            "T|E5 18446744069414584320.000000",
        ]),
    );
}

/// §11.8.2: integral constant expressions keep integer division and unsigned
/// wrapping until the comparison or arithmetic converts their result to real.
#[test]
fn constant_integral_operands() {
    let got = t_lines(
        r#"
module top;
  int unsigned u; int a, b; real r; localparam real RP = 1.5; localparam int unsigned UP = 1;
  localparam real LR1 = (UP - 2) * 1.0;
  localparam real LR2 = 1.5 + 7/2;
  localparam LB1 = (UP - 2) > 1.0;
  initial begin
    u = 1; a = 7; b = 2; r = 1.5;
    $display("T|K1 %f", 1.5 + 7/2);
    $display("T|K2 %0d", 3.5 == 7/2);
    $display("T|K3 %f", (32'd1 - 32'd2) * 1.0);
    $display("T|K4 %f", r + a/b);
    $display("T|K6 %f", 1.5 + a/b);
    $display("T|K7 %0d", r == a/b + 0.5);
    $display("T|K8 %f", (a/b) * 1.0);
    $display("T|K11 %f", 1.0 * (UP - 2));
    $display("T|K12 %f %f %0d", LR1, LR2, LB1);
    $display("T|K14 %0d", (u + 0) > 1.0);
    $display("T|K18 %0d", {u - 2} > 1.0);
    $display("T|K22 %0d", (u * 3) > 1.0);
    $display("T|K23 %0d", (u - 2) * 2 > 1.0);
    // Keep this operand explicitly constant; module-variable optimization
    // differs from a run-time formal (covered by unsigned_negation_359).
    $display("T|K24 %0d", -UP > 1.0);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        sorted(&[
            "T|K1 4.500000",
            "T|K2 0",
            "T|K3 4294967295.000000",
            "T|K4 4.500000",
            "T|K6 4.500000",
            "T|K7 0",
            "T|K8 3.000000",
            "T|K11 4294967295.000000",
            "T|K12 4294967295.000000 4.500000 1",
            "T|K14 0",
            "T|K18 1",
            "T|K22 1",
            "T|K23 1",
            "T|K24 1",
        ]),
    );
}

/// Continuous and nonblocking assignments, `always_comb`, a declaration initializer, function and task arguments and results, compound assignments, class properties and `$rtoi`.
#[test]
fn other_assignment_contexts() {
    let got = t_lines(
        r#"
class C;
  real cr;
  int unsigned cu = 1;
  function void run();
    cr = (cu - 2) * 1.0;
    $display("T|CL1 %f", cr);
    cr = cu - 2;
    $display("T|CL2 %f", cr);
    $display("T|CL3 %0d", (cu - 2) > 1.0);
  endfunction
endclass
module top;
  int unsigned u = 1;
  real ra, rn, rc, rd;
  real ri = u - 2;
  int unsigned w = 32'hFFFF_FFFF;
  assign ra = (u - 2) * 1.0;
  always_comb rc = (u - 2) + 0.25;
  function automatic real f(real x); return x * 2.0; endfunction
  function automatic real g(int unsigned y); real t; t = y - 2; return t; endfunction
  task automatic t1(input real x); $display("T|TK %f", x); endtask
  initial begin
    C c = new;
    real lr;
    byte unsigned b;
    #1;
    rn <= (u - 2) * 1.0;
    #1;
    $display("T|CA %f", ra);
    $display("T|NB %f", rn);
    $display("T|AC %f", rc);
    $display("T|DI %f", ri);
    $display("T|FA %f", f(u - 2));
    $display("T|FR %f", g(1));
    t1(u - 2);
    lr = (u - 2) * 1.0; $display("T|LR %f", lr);
    lr = u - 2; $display("T|LR2 %f", lr);
    b = 1; lr = b - 8'd2; $display("T|LR3 %f", lr);
    $display("T|W %f %f", w * 1.0, w + 1.0);
    $display("T|RT %0d", $rtoi((u - 2) * 1.0) );
    c.run();
    rd = 1.0; rd += u - 2; $display("T|PE %f", rd);
    rd = 2.0; rd *= (u - 2); $display("T|ME %f", rd);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        sorted(&[
            "T|CA 4294967295.000000",
            "T|NB 4294967295.000000",
            "T|AC 4294967295.250000",
            "T|DI 4294967295.000000",
            "T|FA 8589934590.000000",
            "T|FR 4294967295.000000",
            "T|TK 4294967295.000000",
            "T|LR 4294967295.000000",
            "T|LR2 4294967295.000000",
            "T|LR3 255.000000",
            "T|W 4294967295.000000 4294967296.000000",
            "T|RT -1",
            "T|CL1 4294967295.000000",
            "T|CL2 4294967295.000000",
            "T|CL3 1",
            "T|PE 4294967296.000000",
            "T|ME 8589934590.000000",
        ]),
    );
}

/// The same conversions inside a class task.
#[test]
fn class_task_operands() {
    let got = t_lines(
        r#"
class K;
  int unsigned u = 1; int s = 1; byte unsigned ub = 1; real r;
  task automatic run();
    #1;
    r = (u - 2) * 1.0; $display("T|K1 %f", r);
    r = u - 2 + 0.5; $display("T|K2 %f", r);
    r = (ub - 8'd2) * 1.0; $display("T|K3 %f", r);
    r = (s - 2) * 1.0; $display("T|K4 %f", r);
    r = 2.0 - (u - 2); $display("T|K5 %f", r);
    r = (u - 2) / 2.0; $display("T|K6 %f", r);
    r = (u == 1) ? (u - 2) : 1.5; $display("T|K7 %f", r);
    r = u - 2; $display("T|K8 %f", r);
    $display("T|K9 %f", (u - 2) * 1.0);
  endtask
endclass
module top;
  initial begin K k = new; k.run(); end
endmodule
"#,
    );
    assert_eq!(
        got,
        sorted(&[
            "T|K1 4294967295.000000",
            "T|K2 4294967295.500000",
            "T|K3 255.000000",
            "T|K4 -1.000000",
            "T|K5 -4294967293.000000",
            "T|K6 2147483647.500000",
            "T|K7 4294967295.000000",
            "T|K8 4294967295.000000",
            "T|K9 4294967295.000000",
        ]),
    );
}

/// Package, static class and module functions.
#[test]
fn package_class_and_module_functions() {
    let got = t_lines(
        r#"
package pk;
  function automatic void pf(int unsigned u);
    real r;
    $display("T|PF1 %0d", (u - 2) > 1.0);
    r = (u == 1) ? (u - 2) : 1.5; $display("T|PF2 %f", r);
  endfunction
endpackage
class C;
  static function void sf(int unsigned u);
    real r;
    $display("T|SF1 %0d", (u - 2) > 1.0);
    r = (u == 1) ? (u - 2) : 1.5; $display("T|SF2 %f", r);
  endfunction
endclass
module top;
  int unsigned g = 1;
  function automatic void mf(int unsigned u);
    real r;
    $display("T|MF1 %0d", (u - 2) > 1.0);
    r = (u == 1) ? (u - 2) : 1.5; $display("T|MF2 %f", r);
  endfunction
  task automatic mt();
    real r;
    #1;
  endtask
  initial begin
    pk::pf(1); C::sf(1); mf(1); mt();
  end
  always @(g) $display("T|AL1 %0d", (g - 2) > 1.0);
endmodule
"#,
    );
    assert_eq!(
        got,
        sorted(&[
            "T|PF1 1",
            "T|PF2 4294967295.000000",
            "T|SF1 1",
            "T|SF2 4294967295.000000",
            "T|MF1 1",
            "T|MF2 4294967295.000000",
        ]),
    );
}
