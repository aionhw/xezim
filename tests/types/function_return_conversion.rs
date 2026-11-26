//! §13.4.1: a function's value takes its DECLARED return type, converted as
//! an assignment would (§10.7, §6.12.2). A real returned from an integral
//! function stayed real until something assigned it (#324, reported by
//! Dragon-git): `int g(); return 7.6;` printed 7.6, `%h` of a `longint`
//! function printed the f64 bits, and a DPI export handed C the bit pattern.
//! The reverse (an integral value from a real function) stayed integral, and
//! a compiled `fr(x) / 2` divided as integers. Each design runs on every
//! method-execution path; the expected lines are the reference simulator's.

use std::process::Command;

/// The `T|` lines of `src` run with `env` set.
fn t_lines(tag: &str, src: &str, env: &[(&str, &str)]) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_retconv_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("top.sv");
    std::fs::write(&sv, src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .args(["--no-cache", "-s", "top"])
        .arg(&sv)
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

/// The method-execution paths: default (compiled after a few calls),
/// interpreted only, compiled from the first call, and after one call.
const PATHS: &[&[(&str, &str)]] = &[
    &[],
    &[("XEZIM_COMPILE_METHODS", "0")],
    &[("XEZIM_METHOD_TIER", "0")],
    &[("XEZIM_METHOD_TIER", "1")],
];

fn check(tag: &str, src: &str, expected: &[&str]) {
    for (i, env) in PATHS.iter().enumerate() {
        let got = t_lines(&format!("{tag}{i}"), src, env);
        assert_eq!(got, expected, "{tag} with {env:?}");
    }
}

/// Free functions: every integral return type (rounding of .5 and negatives,
/// truncation, signed into unsigned, typedefs), real returns of integral
/// values, and every context a call appears in.
#[test]
fn plain_function_returns() {
    const SRC: &str = r#"`timescale 1ns/1ps
module top;
  typedef enum int {A=0,B=1,C=2,D=3} e_t;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } ps_t;
  typedef int myint_t;
  typedef real myreal_t;
  function int f_int(); return 7.6; endfunction
  function int f_int_neg(); return -7.5; endfunction
  function int f_int_half(); return 2.5; endfunction
  function int f_int_mhalf(); return -2.5; endfunction
  function longint f_long(); return 3.0e10; endfunction
  function shortint f_short(); return 70000.4; endfunction
  function byte f_byte(); return -1.6; endfunction
  function bit [7:0] f_b8(); return -1.4; endfunction
  function logic [11:0] f_l12(); return 4095.6; endfunction
  function logic signed [7:0] f_ls8(); return 127.5; endfunction
  function bit [3:0] f_b4_wide(); return 32'h1234_5678; endfunction
  function int unsigned f_uint(); return -1.0; endfunction
  function e_t f_enum(); return C; endfunction
  function ps_t f_ps(); return 8'hA5 + 0.6; endfunction
  function real f_real_from_int(); return 7; endfunction
  function real f_real_div(); return 7/2; endfunction
  function shortreal f_sr(); return 3; endfunction
  function int f_rvar(); real r; r = 9.5; return r; endfunction
  function int f_fname(); f_fname = 3.7; endfunction
  function int f_time(); time t; t = 100ns; return t / 1ns; endfunction
  function myint_t f_tdint(); return 5.5; endfunction
  function myreal_t f_tdreal(); return 5; endfunction
  function integer f_integer(); return 6.5; endfunction
  function bit signed [15:0] f_s16(); return -300.7; endfunction
  function int unsigned f_su(); int s; s = -5; return s; endfunction
  function int id(int x); return x; endfunction
  function real idr(real x); return x; endfunction
  initial begin
    $display("T|int=%0d neg=%0d half=%0d mhalf=%0d", f_int(), f_int_neg(), f_int_half(), f_int_mhalf());
    $display("T|long=%0d short=%0d byte=%0d b8=%0d l12=%0d ls8=%0d", f_long(), f_short(), f_byte(), f_b8(), f_l12(), f_ls8());
    $display("T|b4w=%0d uint=%0d enum=%0d ps=%h", f_b4_wide(), f_uint(), f_enum(), f_ps());
    $display("T|rint=%f rdiv=%f sr=%f", f_real_from_int(), f_real_div(), f_sr());
    $display("T|rvar=%0d fname=%0d time=%0d tdint=%0d tdreal=%f integer=%0d s16=%0d su=%0d", f_rvar(), f_fname(), f_time(), f_tdint(), f_tdreal(), f_integer(), f_s16(), f_su());
    $display("T|hex int=%h b8=%h long=%h", f_int(), f_b8(), f_long());
    $display("T|arith int+1=%0d int/2=%0d b8+1=%0d cat=%h", f_int() + 1, f_int() / 2, f_b8() + 1, {f_b8(), f_b8()});
    $display("T|cmp eq8=%0d gt7=%0d", f_int() == 8, f_int() > 7);
    $display("T|arg id=%0d idr=%f idrint=%f", id(f_int()), idr(f_int()), idr(f_real_from_int()));
    $display("T|bits b8=%0d int=%0d", $bits(f_b8()), $bits(f_int()));
    $display("T|tern=%0d", 1 ? f_int() : 0);
    void'(f_int());
    $display("T|done");
  end
endmodule
"#;
    check(
        "plain_function_returns",
        SRC,
        &[
            "T|int=8 neg=-8 half=3 mhalf=-3",
            "T|long=30000000000 short=4464 byte=-2 b8=255 l12=0 ls8=-128",
            "T|b4w=8 uint=4294967295 enum=2 ps=a6",
            "T|rint=7.000000 rdiv=3.000000 sr=3.000000",
            "T|rvar=10 fname=4 time=100 tdint=6 tdreal=5.000000 integer=7 s16=-301 su=4294967291",
            "T|hex int=00000008 b8=ff long=00000006fc23ac00",
            "T|arith int+1=9 int/2=4 b8+1=256 cat=ffff",
            "T|cmp eq8=1 gt7=1",
            "T|arg id=8 idr=8.000000 idrint=7.000000",
            "T|bits b8=8 int=32",
            "T|tern=8",
            "T|done",
        ],
    );
}

/// Constant functions, continuous assigns, static/virtual class methods,
/// output and ref formals.
#[test]
fn contexts_and_class_methods() {
    const SRC: &str = r#"`timescale 1ns/1ps
module top;
  function automatic int cf(int a); return a * 1.5; endfunction
  localparam int P = cf(3);
  localparam int Q = cf(5);
  function int g(); return 7.6; endfunction
  wire [31:0] w = g();
  wire [7:0] w2;
  assign w2 = g() + 1;
  class B;
    static function longint s(); return 2.5e1; endfunction
    virtual function int v(); return 1.5; endfunction
    function bit [7:0] m(); return -1.4; endfunction
    static function int sh(); return 0.5; endfunction
  endclass
  class D extends B;
    virtual function int v(); return 2.5; endfunction
  endclass
  task t_out(output int o); o = 2.6; endtask
  task t_outr(output real o); o = 2.6; endtask
  function void f_out(output int o); o = 3.6; endfunction
  function void f_outr(output real o); o = 4.4; endfunction
  function int f_outret(output int o); o = 5.5; return 6.5; endfunction
  function automatic void f_ref(ref int x); x = 4.5; endfunction
  function automatic void f_refr(ref real x); x = x + 0.25; endfunction
  int a, b, c, d, e2; real ra;
  initial begin
    B bb; D dd; B bp;
    bb = new; dd = new; bp = dd;
    #1;
    $display("T|P=%0d Q=%0d w=%0d w2=%0d", P, Q, w, w2);
    $display("T|s=%0d v=%0d vd=%0d m=%0d sh=%0d", B::s(), bb.v(), bp.v(), bb.m(), B::sh());
    $display("T|sh=%h mh=%h", B::s(), bb.m());
    t_out(a); t_outr(b); f_out(c); f_outr(d);
    $display("T|tout=%0d toutr=%0d fout=%0d foutr=%0d", a, b, c, d);
    e2 = f_outret(a);
    $display("T|outret=%0d o=%0d", e2, a);
    a = 1; f_ref(a);
    ra = 1.0; f_refr(ra);
    $display("T|ref=%0d refr=%f", a, ra);
    $display("T|lp=%0d", 1 + g());
    for (int i = 0; i < 2; i++) $display("T|loop%0d=%0d", i, g() + i);
  end
endmodule
"#;
    check(
        "contexts_and_class_methods",
        SRC,
        &[
            "T|P=5 Q=8 w=8 w2=9",
            "T|s=25 v=2 vd=3 m=255 sh=1",
            "T|sh=0000000000000019 mh=ff",
            "T|tout=3 toutr=3 fout=4 foutr=4",
            "T|outret=7 o=6",
            "T|ref=5 refr=1.250000",
            "T|lp=9",
            "T|loop0=8",
            "T|loop1=9",
        ],
    );
}

/// A type-parameter or class-typedef return takes the bound type; a 2-state
/// return maps X/Z to 0.
#[test]
fn type_params_scoped_typedefs_and_two_state() {
    const SRC: &str = r#"`timescale 1ns/1ps
package pk;
  typedef int t_a;
  function t_a pf(); return 4.5; endfunction
endpackage
module top;
  class P #(type T = int);
    T v;
    function T get(); return 2.5; endfunction
    function T getv(); return v; endfunction
    static function T sget(); return 3.5; endfunction
  endclass
  class A; typedef real t; function t f(); return 1.25; endfunction endclass
  class B; typedef int t; function t f(); return 1.75; endfunction endclass
  function real f_real_from_int(); return 7; endfunction
  function real f_rdiv(); int a; a = 7; return a; endfunction
  function int f_x(); return 'x; endfunction
  function logic [7:0] f_lx(); return 8'bx1z0_1010; endfunction
  function bit [7:0] f_bx(); return 8'bx1z0_1010; endfunction
  initial begin
    P #(real) pr; P #(int) pi; P #(shortint) ps; A a; B b;
    pr = new; pi = new; ps = new; a = new; b = new;
    pr.v = 6.25;
    $display("T|pr=%0f pi=%0d ps=%0d prv=%0f", pr.get(), pi.get(), ps.get(), pr.getv());
    $display("T|spr=%0f spi=%0d", P#(real)::sget(), P#(int)::sget());
    $display("T|A=%0f B=%0d", a.f(), b.f());
    $display("T|pkg=%0d", pk::pf());
    $display("T|r2=%0f rdiv2=%0f", f_real_from_int() / 2, f_rdiv() / 2);
    $display("T|x=%0d lx=%b bx=%b", f_x(), f_lx(), f_bx());
  end
endmodule
"#;
    check(
        "type_params_scoped_typedefs_and_two_state",
        SRC,
        &[
            "T|pr=2.500000 pi=3 ps=3 prv=6.250000",
            "T|spr=3.500000 spi=4",
            "T|A=1.250000 B=2",
            "T|pkg=5",
            "T|r2=3.500000 rdiv2=3.500000",
            "T|x=0 lx=x1z01010 bx=01001010",
        ],
    );
}

/// Functions inlined into always blocks and class methods compiled to
/// bytecode (called often enough to tier up).
#[test]
fn compiled_paths() {
    const SRC: &str = r#"`timescale 1ns/1ps
module top;
  function automatic longint fl(input int a); return a * 1.5; endfunction
  function automatic longint fn2(input int a); fn2 = a * 2.5; endfunction
  function automatic int fi(input int a); return a * 1.5; endfunction
  function automatic real fr(input int a); return a; endfunction
  function automatic real frn(input int a); frn = a; endfunction
  function automatic int f2s(input logic [7:0] a); return a; endfunction
  function automatic longint flr(input real a); return a; endfunction
  logic clk = 0;
  int x = 3;
  longint y1, y2, y4; int y3; real r1, r2; int y5; longint y6;
  always_comb y1 = fl(x);
  always_comb y2 = fn2(x);
  always_comb y3 = fi(x) + 1;
  always_comb r1 = fr(x) / 2;
  always_comb r2 = frn(x) / 2;
  always_comb y5 = f2s(8'bx0x0_0001);
  always_comb y6 = flr(x * 0.5) + 1;
  longint q1; int q2; real q3;
  always @(posedge clk) begin
    q1 <= fl(x) + 1;
    q2 <= fi(x) / 2;
    q3 <= fr(x) / 4;
  end
  class C;
    real rp = 2.5;
    logic [31:0] xp = 'x;
    function longint m(int a); return a * 1.5; endfunction
    function int mi(int a); return a * 1.5; endfunction
    function int get_r(); return rp; endfunction
    function longint get_rl(); return rp; endfunction
    function int gx(); return xp; endfunction
    function real mr(int a); return a; endfunction
  endclass
  initial begin
    C c; longint acc; int acci; real accr;
    c = new;
    acc = 0; acci = 0; accr = 0;
    for (int i = 0; i < 50; i++) begin
      acc += c.m(3);
      acci += c.mi(3) + c.get_r();
      acc += c.get_rl();
      accr += c.mr(3) / 2;
    end
    $display("T|cls acc=%0d acci=%0d accr=%f gx=%0d", acc, acci, accr, c.gx());
    $display("T|cls m=%0d mi=%0d getr=%0d getrl=%0d mr=%f", c.m(3), c.mi(3), c.get_r(), c.get_rl(), c.mr(3) / 2);
    #1 clk = 1; #1;
    $display("T|comb y1=%0d y2=%0d y3=%0d r1=%f r2=%f y5=%0d y6=%0d", y1, y2, y3, r1, r2, y5, y6);
    $display("T|ff q1=%0d q2=%0d q3=%f", q1, q2, q3);
    x = 5; #1 clk = 0; #1 clk = 1; #1;
    $display("T|comb2 y1=%0d y2=%0d y3=%0d r1=%f r2=%f y6=%0d", y1, y2, y3, r1, r2, y6);
    $display("T|ff2 q1=%0d q2=%0d q3=%f", q1, q2, q3);
  end
endmodule
"#;
    check(
        "compiled_paths",
        SRC,
        &[
            "T|cls acc=400 acci=400 accr=75.000000 gx=0",
            "T|cls m=5 mi=5 getr=3 getrl=3 mr=1.500000",
            "T|comb y1=5 y2=8 y3=6 r1=1.500000 r2=1.500000 y5=1 y6=3",
            "T|ff q1=6 q2=2 q3=0.750000",
            "T|comb2 y1=8 y2=13 y3=9 r1=2.500000 r2=2.500000 y6=4",
            "T|ff2 q1=9 q2=4 q3=1.250000",
        ],
    );
}

/// Packed struct, enum, 1-bit, unsigned 64-bit (beyond 2^63), integer, time
/// and signed narrow returns.
#[test]
fn edge_types() {
    const SRC: &str = r#"module top;
  typedef struct packed { bit [3:0] a; bit [3:0] b; } bs_t;
  typedef enum logic [1:0] {E0, E1, E2} le_t;
  function shortreal fsr(); return 0.1; endfunction
  function shortreal fsr2(); real r; r = 0.1; return r; endfunction
  function bs_t fbs(); return 8'bx1x1_0011; endfunction
  function le_t fle(); return le_t'(2'bx1); endfunction
  function bit fb(); return 2.6; endfunction
  function logic [3:0] fl4(); return -0.4; endfunction
  function int unsigned fu(); return 4294967295.4; endfunction
  function longint unsigned flu(); return 1.0e19; endfunction
  function integer fint(); return 'z; endfunction
  function time ftm(); return 2.5; endfunction
  function bit signed [3:0] fs4(); return 7.5; endfunction
  initial begin
    $display("T|sr=%.10f sr2=%.10f bs=%b le=%b fb=%0d fl4=%0d fu=%0d flu=%0d fint=%0d ftm=%0d fs4=%0d", fsr(), fsr2(), fbs(), fle(), fb(), fl4(), fu(), flu(), fint(), ftm(), fs4());
  end
endmodule
"#;
    check(
        "edge_types",
        SRC,
        &[
            // §6.12: a shortreal return holds the binary32 value nearest 0.1
            // (the reference simulator keeps binary64 and prints 0.1000000000).
            "T|sr=0.1000000015 sr2=0.1000000015 bs=01010011 le=x1 fb=1 fl4=0 fu=4294967295 flu=10000000000000000000 fint=z ftm=3 fs4=-8",
        ],
    );
}
