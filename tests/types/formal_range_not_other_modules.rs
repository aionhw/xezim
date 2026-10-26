//! §13.4 / §7.4.1: a bit- or part-select of a function FORMAL or LOCAL
//! indexes by the range of ITS declaration. The simulator keeps every
//! subroutine formal and local under its bare name, and the declared
//! ascending range of a vector was looked up in the design-wide
//! `ascending_packed` table — which the formal's registration never
//! touched. So inside `m`, `a[i]` on a `bit [22:0] a` formal consulted the
//! entry of module `test`'s unrelated `reg [0:22] a` and read bit `22-i`:
//! the function returned the bit-reversal of its argument. Conversely an
//! ascending formal (`bit [0:7] b`) had no entry and indexed as if it were
//! descending.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Formal, local and `+:` part-select, each sharing its bare name with an
/// ascending-range signal of the top module; and an ascending formal that
/// must index from ITS msb end.
#[test]
fn formal_and_local_selects_use_own_declared_range() {
    let src = r#"
module m(input bit [22:0] s, output logic [22:0] c_formal, output logic [22:0] c_local,
         output logic [22:0] c_part, output logic [7:0] c_asc);
  // formal bit-select
  function automatic logic [22:0] f_formal(bit [22:0] a);
    logic [22:0] v;
    for (int i = 0; i < 23; i++) v[i] = a[i];
    return v;
  endfunction
  // local-variable bit-select
  function automatic logic [22:0] f_local(bit [22:0] x);
    logic [22:0] a;
    logic [22:0] v;
    a = x;
    for (int i = 0; i < 23; i++) v[i] = a[i];
    return v;
  endfunction
  // indexed part-select of the formal
  function automatic logic [22:0] f_part(bit [22:0] a);
    logic [22:0] v;
    for (int i = 0; i < 23; i++) v[i +: 1] = a[i +: 1];
    return v;
  endfunction
  // an ASCENDING formal: b[0] is its msb (§7.4.1)
  function automatic logic [7:0] f_asc(bit [0:7] b);
    logic [7:0] v;
    for (int i = 0; i < 8; i++) v[i] = b[i];
    return v;
  endfunction
  always @(s) begin
    c_formal = f_formal(s);
    c_local = f_local(s);
    c_part = f_part(s);
    c_asc = f_asc(s[7:0]);
  end
endmodule
module test;
  reg [0:22] a;      // same name as the formals/locals, ascending range
  reg [0:7] b;
  bit [22:0] s;
  wire [22:0] r_formal, r_local, r_part;
  wire [7:0] r_asc;
  m u(.s(s), .c_formal(r_formal), .c_local(r_local), .c_part(r_part), .c_asc(r_asc));
  logic [22:0] g_formal, g_local, g_part;
  logic [7:0] g_asc;
  initial begin
    a = 0; b = 0;
    s = 23'h00032c;
    #1;
    g_formal = r_formal;
    g_local = r_local;
    g_part = r_part;
    g_asc = r_asc;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "g_formal"), 0x00032c, "formal bit-select");
    assert_eq!(u(&sim, "g_local"), 0x00032c, "local bit-select");
    assert_eq!(u(&sim, "g_part"), 0x00032c, "formal `a[i +: 1]`");
    // s[7:0] = 8'h2c = 0010_1100; b[0] is the msb, so v = 0011_0100.
    assert_eq!(
        u(&sim, "g_asc"),
        0x34,
        "ascending formal indexes from its msb"
    );
}

/// The same formal collision from a continuous assignment and an initial
/// block, where the function runs outside any always process.
#[test]
fn formal_select_from_continuous_assign_and_initial() {
    let src = r#"
module m(input bit [7:0] s, output logic [7:0] c);
  function automatic logic [7:0] rev_id(bit [7:0] a);
    logic [7:0] v;
    for (int i = 0; i < 8; i++) v[i] = a[i];
    return v;
  endfunction
  assign c = rev_id(s);
  logic [7:0] d;
  initial begin
    #1 d = rev_id(s);
  end
endmodule
module test;
  reg [0:7] a;
  bit [7:0] s;
  wire [7:0] r;
  m u(.s(s), .c(r));
  logic [7:0] g, gd;
  initial begin
    a = 0;
    s = 8'h2c;
    #2;
    g = r;
    gd = u.d;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("simulate failed");
    assert_eq!(u(&sim, "g"), 0x2c, "continuous assign");
    assert_eq!(u(&sim, "gd"), 0x2c, "initial block");
}
