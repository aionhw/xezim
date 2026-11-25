//! IEEE 1800-2023 Chapter 6.11, 6.12, and 6.21 semantics tests (Milestone 2).
//!
//! Covers:
//! - §6.11: 2-state bit-select & part-select writes (blocking, NBA, always_comb,
//!   module vectors, ascending vectors, packed structs, packed multi-D,
//!   unpacked arrays, class properties) coerce X/Z bits to 0.
//! - §6.12: `shortreal` f32 rounding across scalars, `typedef shortreal`,
//!   1D/2D/3D unpacked arrays, local arrays/queues/associative arrays, local
//!   declaration initializers, unpacked struct members, and class properties
//!   (including parameterized classes).
//! - §6.21: `module automatic`, `program automatic`, `interface automatic`
//!   procedural block locals and subroutines (plus generate blocks, explicit
//!   `static` overrides, and static/automatic child instances); class-method
//!   `static` fixed (1D/2D), dynamic, queue, and associative arrays including
//!   parameterized specializations.

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 500).unwrap_or_else(|e| panic!("simulation failed: {e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn two_state_bit_and_part_select_drop_xz() {
    let src = r#"
class Bag;
  bit [7:0] p;
  bit [1:0][3:0] mp;
  function new();
    p = 8'hFF;
    mp[0] = 4'hF;
    mp[1] = 4'hF;
  endfunction
  function void corrupt_this(logic xbit, logic [1:0] xz2);
    this.p[1] = xbit;
    this.p[5:4] = xz2;
    this.mp[1][2] = xbit;
    this.mp[0][3:2] = xz2;
  endfunction
endclass

module top;
  typedef struct packed {
    bit [3:0] hi;
    bit [3:0] lo;
  } ps_t;

  bit [7:0] b8;
  bit [0:7] b8_asc;
  bit [1:0][3:0] p2d;
  ps_t ps;
  bit [3:0] u1[2];
  bit [3:0] u2[2][2];
  bit [7:0] comb_in;
  bit [7:0] comb_out;
  logic [7:0] comb_src;
  bit [7:0] nb8;
  bit [3:0] nu1[2];

  logic x1 = 1'bx;
  logic z1 = 1'bz;
  logic [3:0] xz4 = 4'b1x0z;
  int idx;

  always_comb begin
    comb_out = comb_in;
    comb_out[3] = comb_src[3];
    comb_out[7:6] = comb_src[7:6];
  end

  initial begin
    Bag bag;
    b8 = 8'hFF;
    b8[3] = x1;
    b8[2] = z1;
    b8[7:4] = xz4;
    idx = 0;
    b8[idx +: 2] = 2'b1z;

    b8_asc = 8'hFF;
    b8_asc[1] = x1;

    p2d[0] = 4'hF;
    p2d[1] = 4'b1z0x;
    p2d[0][2] = x1;
    p2d[0][1:0] = 2'b1z;

    ps = 8'hFF;
    ps.hi[2] = x1;
    ps.lo[1:0] = 2'b1z;

    u1[0] = 4'hF;
    u1[1] = 4'hF;
    u1[0][1] = x1;
    u1[1][3:2] = 2'b1z;

    u2[0][1] = 4'b1x0z;
    u2[1][0] = 4'hF;
    u2[1][0][2] = z1;

    comb_in = 8'hFF;
    comb_src = 8'b1x_00_z0_00;

    nb8 <= 8'hFF;
    nu1[0] <= 4'hF;
    #1;
    nb8[1] <= x1;
    nb8[5:4] <= 2'b1z;
    nu1[0][0] <= x1;
    nu1[0][3:2] <= 2'b1z;

    bag = new();
    bag.corrupt_this(x1, 2'b1z);
    bag.p[7] = z1;
    bag.p[3:2] = 2'b1x;

    #1;
    $display("S1|%b|%b|%0b", b8, b8_asc, !$isunknown(b8) && !$isunknown(b8_asc));
    $display("S2|%b|%b|%0b", p2d[1], p2d[0], !$isunknown(p2d));
    $display("S3|%b|%b|%b|%b|%0b", ps, u1[0], u1[1], u2[0][1], u2[1][0] === 4'b1011);
    $display("S4|%b|%b|%b|%0b", comb_out, nb8, nu1[0], !$isunknown(comb_out) && !$isunknown(nb8) && !$isunknown(nu1[0]));
    $display("S5|%b|%b|%b|%0b", bag.p, bag.mp[1], bag.mp[0], !$isunknown(bag.p) && !$isunknown(bag.mp));
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "S1|10000010|10111111|1",
            "S2|1000|1010|1",
            "S3|10111110|1101|1011|1000|1",
            "S4|10110111|11101101|1010|1",
            "S5|01101001|1011|1011|1",
        ]
    );
}

#[test]
fn shortreal_f32_rounding_every_storage_class() {
    let src = r#"
typedef shortreal sr_t;
typedef sr_t sr2_t;

typedef struct {
  sr_t s;
  real d;
} st_sr_t;

class Holder #(type T = real);
  T v;
  st_sr_t st;
  function void set_val(real x);
    v = x;
    st.s = x;
    st.d = x;
  endfunction
endclass

module top;
  shortreal s_direct;
  sr2_t s_typedef;
  real d_full;

  sr2_t arr1[2];
  sr2_t arr2[2][2];
  sr2_t arr3[2][2][2];
  st_sr_t st_mod;

  function automatic bit check_sr_bits(real x);
    return ($realtobits(x) == 64'h3fd5555560000000);
  endfunction

  initial begin
    sr2_t loc_init = 1.0 / 3.0;
    sr2_t loc_assign;
    sr2_t loc_arr[2];
    sr2_t loc_q[$];
    sr2_t loc_aa[int];
    st_sr_t st_loc;
    Holder#(shortreal) h_sr;
    Holder#(sr2_t) h_td;
    Holder#(real) h_re;

    s_direct = 1.0 / 3.0;
    s_typedef = 1.0 / 3.0;
    d_full = 16777217;

    arr1[0] = 1.0 / 3.0;
    arr1[1] = 16777217;
    arr2[1][0] = 1.0 / 3.0;
    arr2[0][1] = 16777217;
    arr3[1][1][1] = 1.0 / 3.0;
    arr3[0][0][1] = 16777217;

    loc_assign = 16777217;
    loc_arr[0] = 1.0 / 3.0;
    loc_arr[1] = 16777217;
    loc_q.push_back(1.0 / 3.0);
    loc_q.push_back(16777217);
    loc_aa[7] = 1.0 / 3.0;
    loc_aa[9] = 16777217;

    st_mod.s = 1.0 / 3.0;
    st_mod.d = 16777217;
    st_loc.s = 16777217;
    st_loc.d = 16777217;

    h_sr = new();
    h_td = new();
    h_re = new();
    h_sr.set_val(16777217);
    h_td.set_val(1.0 / 3.0);
    h_re.set_val(16777217);

    $display("SR1|%0d|%0d|%0d|%0.0f",
      $shortrealtobits(s_direct) == 32'h3eaaaaab,
      $shortrealtobits(s_typedef) == 32'h3eaaaaab,
      $shortrealtobits(loc_init) == 32'h3eaaaaab,
      d_full);
    $display("SR2|%0d|%0.0f|%0d|%0.0f|%0d|%0.0f",
      check_sr_bits(arr1[0]), real'(arr1[1]),
      check_sr_bits(arr2[1][0]), real'(arr2[0][1]),
      check_sr_bits(arr3[1][1][1]), real'(arr3[0][0][1]));
    $display("SR3|%0.0f|%0d|%0.0f|%0d|%0.0f|%0d|%0.0f",
      real'(loc_assign),
      check_sr_bits(loc_arr[0]), real'(loc_arr[1]),
      check_sr_bits(loc_q[0]), real'(loc_q[1]),
      check_sr_bits(loc_aa[7]), real'(loc_aa[9]));
    $display("SR4|%0d|%0.0f|%0.0f|%0.0f",
      check_sr_bits(st_mod.s), st_mod.d, real'(st_loc.s), st_loc.d);
    $display("SR5|%0.0f|%0d|%0d|%0.0f|%0.0f",
      real'(h_sr.v), check_sr_bits(h_td.v), check_sr_bits(h_td.st.s), h_sr.st.d, h_re.v);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "SR1|1|1|1|16777217",
            "SR2|1|16777216|1|16777216|1|16777216",
            "SR3|16777216|1|16777216|1|16777216|1|16777216",
            "SR4|1|16777217|16777216|16777217",
            "SR5|16777216|1|1|16777217|16777217",
        ]
    );
}

#[test]
fn automatic_modules_programs_and_interfaces() {
    let src = r#"
interface automatic auto_ifc;
  function int fact(int n);
    int r = (n <= 1) ? 1 : n * fact(n - 1);
    return r;
  endfunction
  function int bump_static();
    static int s = 10;
    s++;
    return s;
  endfunction
endinterface

program automatic auto_prog(input int seed);
  function int tri_sum(int n);
    int acc = 0;
    if (n <= 0) return 0;
    acc = n + tri_sum(n - 1);
    return acc;
  endfunction
  initial begin
    int s0;
    s0 = tri_sum(seed);
    $display("PROG|%0d", s0);
    #15;
  end
endprogram

module sub_static(input logic trig);
  always @(posedge trig) begin
    bit [31:0] cnt;
    automatic int au = 0;
    cnt++;
    au++;
    $display("SUB_ST|%0d|%0d", cnt, au);
  end
endmodule

module automatic sub_auto(input logic trig);
  always @(posedge trig) begin
    bit [31:0] bare;
    int loc = 0;
    static int st = 0;
    bare++;
    loc++;
    st++;
    $display("SUB_AU|%0d|%0d|%0d", bare, loc, st);
  end
  if (1) begin : g
    function int gf(int n);
      int k = 1;
      if (n <= 1) return k;
      return n * gf(n - 1);
    endfunction
  end
endmodule

module automatic top;
  logic trig_top = 0;
  logic trig_st = 0;
  logic trig_au = 0;
  auto_ifc u_ifc();
  auto_prog u_prog(4);
  sub_static u_st(trig_st);
  sub_auto u_au(trig_au);

  function int fact(int n);
    int t = 1;
    if (n <= 1) return t;
    t = n * fact(n - 1);
    return t;
  endfunction

  always @(posedge trig_top) begin
    int blk_auto = 5;
    static int blk_st = 5;
    blk_auto++;
    blk_st++;
    $display("TOP_ALW|%0d|%0d", blk_auto, blk_st);
  end

  initial begin
    for (int i = 0; i < 3; i++) begin
      int x = 0;
      static int sx = 0;
      x++;
      sx++;
      $display("LOOP|%0d|%0d", x, sx);
    end
    $display("CALLS|%0d|%0d|%0d|%0d|%0d",
      fact(5), u_ifc.fact(4), u_ifc.bump_static(), u_ifc.bump_static(), u_au.g.gf(4));
    #1 trig_top = 1;
    #1 trig_st = 1;
    #1 trig_au = 1;
    #1 trig_top = 0; trig_st = 0; trig_au = 0;
    #1 trig_top = 1;
    #1 trig_st = 1;
    #1 trig_au = 1;
    #1;
  end

  final begin
    int fv = 42;
    $display("FINAL|%0d", fv);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "LOOP|1|1",
            "LOOP|1|2",
            "LOOP|1|3",
            "CALLS|120|24|11|12|24",
            "PROG|10",
            "TOP_ALW|6|6",
            "SUB_ST|1|1",
            "SUB_AU|1|1|1",
            "TOP_ALW|6|7",
            "SUB_ST|2|1",
            "SUB_AU|1|1|2",
            "FINAL|42",
        ]
    );
}

#[test]
fn class_method_static_collections() {
    let src = r#"
typedef enum logic [1:0] { RED = 0, GREEN = 1, BLUE = 2 } color_e;

class Worker;
  function int step(int x, color_e c);
    static int fixed1d[2] = '{10, 20};
    static int fixed2d[2][2] = '{'{1, 2}, '{3, 4}};
    static int q[$];
    static int dyn[];
    static int aa[string];
    static int eaa[color_e];
    string ename;

    fixed1d[0] += x;
    fixed2d[1][0] += x;
    q.push_back(x);
    if (dyn.size() == 0) begin
      dyn = new[2];
      dyn[0] = 100;
      dyn[1] = 200;
    end
    dyn[0] += x;
    if (!aa.exists("sum")) aa["sum"] = 0;
    aa["sum"] += x;
    if (!eaa.exists(c)) eaa[c] = 0;
    eaa[c] += 1;
    ename = "";
    foreach (eaa[k]) begin
      if (k == c) ename = k.name();
    end
    $display("W|%0d|%0d|%0d|%0d|%0d|%0d|%s",
      fixed1d[0], fixed2d[1][0], q.size(), dyn[0], aa["sum"], eaa[c], ename);
    return q.size();
  endfunction
endclass

class Spec #(parameter int ID = 0);
  static function int bump(int v);
    static int sq[$];
    static int sarr[2] = '{ID, 0};
    sq.push_back(ID * 10 + v);
    sarr[1] += v;
    return sq.size() * 1000 + sq[0] * 10 + sarr[1];
  endfunction
endclass

module top;
  function int helper_collision();
    static int fixed1d[2] = '{99, 88};
    fixed1d[0]++;
    return fixed1d[0];
  endfunction

  initial begin
    Worker w1 = new();
    Worker w2 = new();
    void'(helper_collision());
    void'(w1.step(3, RED));
    void'(w2.step(5, RED));
    void'(w1.step(2, GREEN));
    $display("COL|%0d", helper_collision());
    $display("SP|%0d|%0d|%0d|%0d",
      Spec#(1)::bump(1), Spec#(2)::bump(2), Spec#(1)::bump(3), Spec#(2)::bump(4));
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "W|13|6|1|103|3|1|RED",
            "W|18|11|2|108|8|2|RED",
            "W|20|13|3|110|10|1|GREEN",
            "COL|101",
            "SP|1111|1222|2114|2226",
        ]
    );
}
