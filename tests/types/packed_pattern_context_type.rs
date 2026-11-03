//! IEEE 1800-2023 §10.8, §10.9: an assignment pattern takes the type of its
//! assignment-like context, and a typed pattern `T'{...}` keeps `T`. Each item
//! is evaluated as an assignment to the member or element it lands in
//! (§10.9.1, §10.9.2).

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn has(out: &[String], line: &str) {
    assert!(out.iter().any(|l| l == line), "missing {line:?} in {out:?}");
}

/// Every assignment-like context of a packed struct places the first item
/// in the first (most significant) member.
#[test]
fn packed_struct_pattern_in_every_context() {
    let out = lines(
        r#"
module tb;
  typedef struct packed {bit a; bit b;} s_t;
  function s_t f_ret(); return '{1,0}; endfunction
  function bit [1:0] f_arg(s_t s); return s; endfunction
  function bit [1:0] f_def(s_t s = '{1,0}); return s; endfunction
  function automatic bit [1:0] f_loc(); s_t l = '{1,0}; return l; endfunction
  class C; s_t p = '{1,0}; endclass
  s_t nba, ca, m, par;
  logic c;
  assign ca = '{1,0};
  initial begin
    static s_t s = '{1,0};
    automatic s_t a = '{1,0};
    C o = new;
    c = 1;
    m = c ? '{1,0} : '{0,1};
    par = ('{1,0});
    nba <= '{1,0};
    #1;
    $display("static=%b auto=%b local=%b ret=%b arg=%b def=%b", s, a, f_loc(), f_ret(),
             f_arg('{1,0}), f_def());
    $display("prop=%b nba=%b ca=%b cond=%b paren=%b", o.p, nba, ca, m, par);
  end
endmodule
"#,
    );
    has(&out, "static=10 auto=10 local=10 ret=10 arg=10 def=10");
    has(&out, "prop=10 nba=10 ca=10 cond=10 paren=10");
}

/// §11.4.11: an x condition merges both patterns bit by bit.
#[test]
fn conditional_pattern_with_unknown_condition_merges() {
    let out = lines(
        r#"
module tb;
  typedef struct packed {logic a; logic b;} s_t;
  s_t m;
  logic c;
  initial begin
    c = 1'bx;
    m = c ? '{1,0} : '{1,1};
    $display("m=%b", m);
  end
endmodule
"#,
    );
    has(&out, "m=1x");
}

/// §10.9.1: a packed vector is an array of bits; a multi-dimensional packed
/// array takes one dimension per nesting level.
#[test]
fn packed_array_patterns() {
    let out = lines(
        r#"
module tb;
  initial begin
    logic [7:0] z = '{default:1};
    bit [1:0][3:0] y = '{4,5};
    bit [1:0][1:0][1:0] y3 = '{'{1,2},'{3,0}};
    bit [1:0][3:0] yd = '{default:1};
    $display("z=%h y=%h y3=%b yd=%h", z, y, y3, yd);
  end
endmodule
"#,
    );
    has(&out, "z=ff y=45 y3=01101100 yd=11");
}

/// §10.9.2: member names, type keys, and `default:` reaching the members
/// of a nested struct; real items round (§6.12.2).
#[test]
fn struct_pattern_keys() {
    let out = lines(
        r#"
module tb;
  typedef struct packed {bit a; bit b;} s_t;
  typedef struct packed {logic [3:0] hi; s_t inner; logic [1:0] lo;} n_t;
  initial begin
    n_t n = '{lo: 2'd2, hi: 4'ha, inner: '{1,1}};
    n_t nd = '{hi: 4'h3, default: 1};
    struct packed {logic [7:0] a; bit b; bit signed [31:0] c;} tk = '{int:1, default:0};
    s_t rr = '{0.6, 0.4};
    $display("n=%b nd=%b tk=%h_%b_%h rr=%b", n, nd, tk.a, tk.b, tk.c, rr);
  end
endmodule
"#,
    );
    has(&out, "n=10101110 nd=00111101 tk=00_0_00000001 rr=10");
}

/// A packed union writes each member over the same bits, in order.
#[test]
fn packed_union_pattern() {
    let out = lines(
        r#"
module tb;
  typedef union packed {logic [3:0] u; logic [3:0] v;} u_t;
  initial begin
    u_t u = '{4'h5, 4'h9};
    $display("u=%h", u);
  end
endmodule
"#,
    );
    has(&out, "u=9");
}

/// §10.9: `T'{...}` has `T`'s value in any context, including a package
/// scoped type, a cast operand and the LRM's concatenation example.
#[test]
fn typed_patterns_keep_their_type() {
    let out = lines(
        r#"
package pkg;
  typedef struct packed {bit a; bit b;} ps_t;
endpackage
module tb;
  typedef struct packed {bit a; bit b;} s_t;
  typedef logic [1:0][3:0] T;
  initial begin
    $display("typed=%b pkg=%b cast=%b lrm=%h", s_t'{1,0}, pkg::ps_t'{1,0}, s_t'('{1,0}),
             shortint'({T'{1,2}, T'{3,4}}));
  end
endmodule
"#,
    );
    has(&out, "typed=10 pkg=10 cast=10 lrm=1234");
}

/// A typed pattern assigned to an unpacked struct places its items member
/// by member, like an untyped one.
#[test]
fn typed_pattern_into_unpacked_struct() {
    let out = lines(
        r#"
module tb;
  typedef struct packed {bit a; bit b;} s_t;
  typedef struct {int x; s_t y;} us_t;
  us_t us, un;
  initial begin
    us = us_t'{5, '{1,0}};
    un <= us_t'{7, '{0,1}};
    #1 $display("us=%0d %b un=%0d %b", us.x, us.y, un.x, un.y);
  end
endmodule
"#,
    );
    has(&out, "us=5 10 un=7 01");
}

/// A signed packed struct pattern is signed (§7.2.1); `default:` is
/// evaluated as an assignment to each member, at that member's width; a
/// 2-state member takes x as 0; index keys follow the declared range;
/// replication repeats the items; type keys and `default:` descend into a
/// nested struct.
#[test]
fn pattern_member_conversions() {
    let out = lines(
        r#"
module tb;
  typedef struct packed signed {logic [3:0] a; logic [3:0] b;} ss_t;
  typedef struct packed {logic [7:0] a; logic [8:0] b;} w_t;
  typedef struct packed {bit b; logic l;} bl_t;
  typedef struct packed {logic [3:0] x; int y;} in_t;
  typedef struct packed {in_t i; logic [7:0] z;} out_t;
  int i;
  logic [7:0] a, b;
  w_t w;
  bl_t bl;
  bit [0:3][1:0] asc;
  bit [3:0][1:0] desc;
  bit [3:0][1:0] rep;
  out_t o;
  initial begin
    i = ss_t'{a: -1, default: 0};
    a = 8'd3; b = 8'd255;
    w = '{default: (a + b) >> 1};
    bl = '{1'bx, 1'bx};
    asc = '{0: 2'd1, default: 0};
    desc = '{0: 2'd1, default: 0};
    rep = '{2{2'd1, 2'd2}};
    o = '{int: 7, default: 4'h3};
    $display("i=%0d w=%h_%h bl=%b", i, w.a, w.b, bl);
    $display("asc=%b desc=%b rep=%b o=%h", asc, desc, rep, o);
  end
endmodule
"#,
    );
    has(&out, "i=-16 w=01_081 bl=0x");
    has(
        &out,
        "asc=01000000 desc=00000001 rep=01100110 o=30000000703",
    );
}

/// §7.3.1: bits of a soft packed union above the member a pattern sets keep
/// the union's uninitialized value: x when 4-state, 0 when 2-state.
#[test]
fn soft_union_pattern_padding() {
    let out = run_bin(
        r#"
module tb;
  typedef union soft packed {logic [7:0] w; logic [3:0] n;} su_t;
  typedef union soft packed {bit [7:0] w; bit [3:0] n;} su2_t;
  initial $display("su=%b su2=%b", su_t'{n: 4'h5}, su2_t'{n: 4'h5});
endmodule
"#,
        "soft_union",
        &[],
    );
    assert!(out.contains("su=xxxx0101 su2=00000101"), "{out}");
}

/// `$typename` and `%p` of a typed pattern see its type (§20.6.1,
/// §21.2.1.7).
#[test]
fn typed_pattern_type_queries() {
    let out = lines(
        r#"
module tb;
  typedef struct packed {logic [3:0] a; logic [3:0] b;} s_t;
  s_t v;
  initial begin
    v = '{1, 2};
    $display("%p|%p|%s", v, s_t'{a: 1, b: 2}, $typename(s_t'{1, 2}));
  end
endmodule
"#,
    );
    let line = out.iter().find(|l| l.contains('|')).expect("output");
    let parts: Vec<&str> = line.split('|').collect();
    assert_eq!(parts[0], parts[1], "{line}");
    assert!(parts[2].contains("s_t"), "{line}");
}

/// A typed pattern names a type parameter of a class (§8.25) or a type
/// declared in a block, which shadows the module's (§6.18).
#[test]
fn typed_pattern_scoped_types() {
    let out = lines(
        r#"
typedef struct packed {logic [3:0] a; logic [3:0] b;} p_t;
module tb;
  typedef struct packed {logic [1:0] a; logic [1:0] b;} t_t;
  class C #(type T = int);
    function T mk(); return T'{default: 1}; endfunction
    function T mk2(); return '{default: 1}; endfunction
  endclass
  C #(p_t) c;
  initial begin
    c = new;
    $display("j=%h %h", c.mk(), c.mk2());
    begin
      typedef struct packed {logic [7:0] a; logic [7:0] b;} t_t;
      t_t l = '{1, 2};
      $display("k=%h %h", l, t_t'{3, 4});
    end
  end
endmodule
"#,
    );
    has(&out, "j=11 11");
    has(&out, "k=0102 0304");
}

fn run_bin(src: &str, tag: &str, env: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_pattern_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("tb.sv");
    std::fs::write(&path, src).expect("write");
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"));
    c.current_dir(&dir).arg("--no-cache").arg(&path);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Typed and untyped patterns in a clocked process compile: no construct
/// is handed to the interpreter. A pattern actual of an inlined call takes
/// the formal's type, not the assignment target's.
#[test]
fn clocked_pattern_assignments_compile() {
    const SRC: &str = r#"
module tb;
  typedef struct packed {bit a; bit b;} s_t;
  typedef struct packed {logic [3:0] a; logic [3:0] b;} w_t;
  function automatic w_t f(s_t s); return {6'b0, s}; endfunction
  logic clk = 0;
  logic [7:0] d = 8'h12;
  w_t q, t, u;
  s_t r;
  logic c = 1;
  always #1 clk = ~clk;
  always_ff @(posedge clk) begin
    q <= f('{1, 0});
    r <= c ? '{1, 0} : '{0, 1};
    t <= w_t'{a: d[3:0], b: ~d[3:0]};
    u <= '{a: d[7:4], default: 4'h9};
  end
  initial begin #2 $display("q=%h r=%b t=%h u=%h", q, r, t, u); $finish; end
endmodule
"#;
    let out = run_bin(SRC, "compiled", &[("XEZIM_FALLBACK_SITES", "1")]);
    assert!(out.contains("q=02 r=10 t=2d u=19"), "{out}");
    assert!(!out.contains("[FALLBACK]"), "{out}");
}

/// A checkpoint taken while a task is suspended restores; the function it
/// calls afterwards returns a pattern in its return type.
#[test]
fn pattern_return_after_restore() {
    const SRC: &str = r#"
module tb;
  typedef struct packed {bit a; bit b;} s_t;
  function automatic s_t f(bit x); return '{x, ~x}; endfunction
  task automatic t();
    #20;
    $display("R %b", f(1'b1));
  endtask
  initial begin t(); $finish; end
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("xezim_pattern_ckpt_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let ck = dir.join("c.ckpt");
    let spec = format!("10ns,{}", ck.display());
    let saved = run_bin(SRC, "ckpt_save", &[]);
    assert!(saved.contains("R 10"), "{saved}");
    let path = dir.join("tb.sv");
    std::fs::write(&path, SRC).expect("write");
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .arg("--no-cache")
            .args(args)
            .arg(&path)
            .output()
            .expect("run xezim");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    run(&["--save-at", &spec]);
    let restored = run(&["--restore", ck.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(restored.contains("R 10"), "{restored}");
}

/// A typed pattern in an instance or generate body reads that scope's
/// names and types: ports, genvars, type parameters and local typedefs.
#[test]
fn typed_pattern_in_instance_scope() {
    let out = lines(
        r#"
typedef struct packed {logic [3:0] a, b;} s_t;
module leaf(input s_t i, output s_t o); assign o = i; endmodule
module sub #(parameter type T = logic [7:0]) (input logic [3:0] pi, output s_t o,
                                              output T t1, output T t2);
  logic [3:0] x = 4'h3, y = 4'h4;
  s_t l;
  assign o = s_t'{4'h1, pi};
  assign t1 = T'{a: 1, b: 2};
  assign t2 = T'{default: '1};
  leaf lf(.i(s_t'{x, y}), .o(l));
endmodule
module loc1(output logic [7:0] o);
  typedef struct packed {logic [1:0] p; logic [5:0] q;} l_t;
  assign o = l_t'{p: 2'h3, q: 6'h1};
endmodule
module loc2(output logic [7:0] o);
  typedef struct packed {logic [5:0] p; logic [1:0] q;} l_t;
  assign o = l_t'{p: 6'h3, q: 2'h1};
endmodule
module tb;
  logic [3:0] pi = 4'hE, x = 4'hA, y = 4'hB;
  s_t o, t1, t2, g[2];
  logic [7:0] l1, l2;
  sub #(.T(s_t)) u(.pi(4'h2), .o(o), .t1(t1), .t2(t2));
  loc1 u1(.o(l1));
  loc2 u2(.o(l2));
  for (genvar i = 0; i < 2; i++) begin : G
    assign g[i] = s_t'{i[3:0], 4'h7};
  end
  initial #1 $display("o=%h lf=%h t=%h %h g=%h %h l=%h %h", o, u.l, t1, t2, g[0], g[1], l1, l2);
endmodule
"#,
    );
    has(&out, "o=12 lf=34 t=12 ff g=07 17 l=c1 0d");
}

/// §10.9: a typed pattern of an unpacked type holds the value a variable of
/// that type would; in an assignment-like context its items place as an
/// untyped pattern's do.
#[test]
fn unpacked_typed_patterns() {
    let out = lines(
        r#"
typedef struct {int a, b;} s_t;
typedef int arr_t[3];
typedef int arr2_t[2];
typedef int q_t[$];
class C; s_t s = s_t'{1, 2}; endclass
module leaf(input arr2_t ai, input s_t si);
  initial #2 $display("port %0d %0d %0d", ai[0], ai[1], si.a + si.b);
endmodule
module tb;
  s_t s = '{4, 3}, w;
  arr_t ar = '{1, 2, 3}, r;
  int d[] = q_t'{1, 2, 3};
  q_t q;
  assign w = s_t'{7, 8};
  leaf l(.ai(arr2_t'{9, 10}), .si(s_t'{1, 2}));
  function q_t fq(); return q_t'{1, 2, 3}; endfunction
  function arr_t fa(); return arr_t'{4, 5, 6}; endfunction
  initial begin
    C c = new;
    #1;
    $display("eq %b %b %b", s == s_t'{4, 3}, s != s_t'{4, 3}, ar == arr_t'{1, 2, 3});
    $display("w %0d %0d prop %0d %0d", w.a, w.b, c.s.a, c.s.b);
    q = fq(); r = fa();
    $display("q %p r %p d %0d", q, r, d.size());
    $display("bits %0d p %p", $bits(arr_t'{1, 2, 3}), arr_t'{1, 2, 3});
  end
endmodule
"#,
    );
    has(&out, "eq 1 0 1");
    has(&out, "w 7 8 prop 1 2");
    has(&out, "q '{1, 2, 3} r '{4, 5, 6} d 3");
    has(&out, "bits 96 p '{1, 2, 3}");
    has(&out, "port 9 10 3");
    assert!(!out.iter().any(|l| l.contains("uncovered")), "{out:?}");
}

/// A typed row of an unpacked type inside a pattern is a row; a typed
/// element is one value for every leaf a `default:` reaches.
#[test]
fn unpacked_typed_rows() {
    let out = lines(
        r#"
typedef int row_t[3];
typedef struct {int a, b;} us_t;
module tb;
  int m [2][3] = '{default: row_t'{4, 5, 6}};
  int n [2][3] = '{row_t'{1, 2, 3}, '{default: 9}};
  us_t sa [2][2] = '{default: us_t'{7, 8}};
  initial #1 $display("m %0d %0d %0d n %0d %0d %0d sa %0d %0d", m[0][0], m[1][1], m[1][2],
                      n[0][0], n[0][2], n[1][1], sa[1][0].a, sa[1][1].b);
endmodule
"#,
    );
    has(&out, "m 4 5 6 n 1 3 9 sa 7 8");
}

/// §10.9.1, §10.9.2: `default:` assigns a value whose self-determined type
/// matches the element or member whole; otherwise it descends.
#[test]
fn default_of_matching_type_assigns_whole() {
    let out = lines(
        r#"
typedef struct packed {logic [3:0] a, b;} e_t;
package pk; localparam e_t PR = '{4'h3, 4'hC}; endpackage
module sub; localparam e_t SR = '{4'h7, 4'h1}; e_t [1:0] s;
  initial begin s = '{default: SR}; #1 $display("sub %h", s); end
endmodule
module tb;
  localparam e_t RST = '{4'hA, 4'h5};
  e_t v = '{4'h9, 4'h6};
  e_t [3:0] x, y, z, c;
  e_t [1:0] p;
  logic [3:0][7:0] w = 32'h01020304;
  logic [31:0] f = 32'h01020304;
  logic [1:0][3:0][7:0] m3, m4;
  sub u();
  assign y = '{default: RST};
  initial begin
    x = '{default: RST}; z = '{default: v}; c = '{default: e_t'(8'hB4)};
    p = '{default: pk::PR}; m3 = '{default: w}; m4 = '{default: f};
    #1 $display("%h %h %h %h %h %h %h", x, y, z, c, p, m3, m4);
  end
endmodule
"#,
    );
    has(
        &out,
        "a5a5a5a5 a5a5a5a5 96969696 b4b4b4b4 3c3c 0102030401020304 0404040404040404",
    );
    has(&out, "sub 7171");
}

/// Compiled processes lay a pattern over the declared type, multi-dimensional
/// members included, and agree with the interpreter on `default:`.
#[test]
fn compiled_patterns_use_declared_types() {
    const SRC: &str = r#"
typedef struct packed {logic [3:0] h; logic [1:0][3:0] v;} s_t;
typedef struct packed {s_t in; logic [3:0] t;} o_t;
module sub(input logic [3:0] d, output s_t o, output o_t p);
  always_comb o = '{h: d, v: '{4'h2, d}};
  always_comb begin p.t = d; p.in = '{h: d, v: '{4'h2, d}}; end
endmodule
module tb;
  logic clk = 0;
  s_t a, arr[2], o;
  o_t p;
  logic [3:0] d = 4'h5;
  logic [511:0] q;
  sub u(.d(d), .o(o), .p(p));
  always @(posedge clk) a <= '{h: d, v: '{4'h2, d}};
  always_comb arr[0] = '{h: d, v: '{4'h2, d}};
  always_comb arr[1] = '{default: d};
  always_ff @(posedge clk) q <= '{default: d};
  initial begin
    #1 clk = 1;
    #1 $display("a=%h arr=%h %h o=%h p=%h q=%h", a, arr[0], arr[1], o, p, q[7:0]);
    $finish;
  end
endmodule
"#;
    let out = run_bin(SRC, "declared", &[("XEZIM_FALLBACK_SITES", "1")]);
    assert!(out.contains("a=525 arr=525 555 o=525 p=5255 q=ff"), "{out}");
    assert!(!out.contains("[FALLBACK]"), "{out}");
}

/// §10.9.1, §10.9.2: an uncovered member is an error; a compiled process
/// leaves the pattern to the interpreter, which warns and sets it to 0.
#[test]
fn compiled_uncovered_pattern_warns() {
    const SRC: &str = r#"
module tb;
  typedef struct packed {logic [3:0] a, b;} s_t;
  logic clk = 0;
  s_t q;
  always_ff @(posedge clk) q <= '{a: 4'h1};
  initial begin #1 clk = 1; #1 $display("q=%h", q); $finish; end
endmodule
"#;
    let out = run_bin(SRC, "uncovered", &[]);
    assert!(out.contains("q=10"), "{out}");
    assert!(out.contains("uncovered"), "{out}");
}

/// §10.9: a packed typed pattern is a value of its own type, whatever
/// target or container it goes to.
#[test]
fn packed_typed_pattern_keeps_type_in_any_target() {
    let out = lines(
        r#"
typedef struct packed {logic [1:0] h; logic [5:0] l;} hl_t;
class K; logic [7:0] v[2]; endclass
module tb;
  mailbox #(hl_t) mb = new;
  hl_t g;
  logic [15:0] w;
  initial begin
    K k = new;
    k.v[0] = hl_t'{1, 2};
    k.v[1] <= hl_t'{1, 2};
    w = hl_t'{1, 2};
    mb.put(hl_t'{2, 3});
    mb.get(g);
    #1 $display("%h %h %h %h", k.v[0], k.v[1], w, g);
  end
endmodule
"#,
    );
    has(&out, "42 42 0042 83");
}

/// §10.9.2: type and default keys reach a packed member of an unpacked
/// struct; a bare type name is a type key.
#[test]
fn default_reaches_packed_member_of_unpacked_struct() {
    let out = lines(
        r#"
typedef struct packed {logic [3:0] a, b;} ps_t;
typedef struct {ps_t p; int x;} us_t;
module tb;
  us_t u, t;
  initial begin
    u = '{default: 1};
    t = '{ps_t: 8'hA5, default: 3};
    #1 $display("%h %0d %h %0d", u.p, u.x, t.p, t.x);
  end
endmodule
"#,
    );
    has(&out, "11 1 a5 3");
}
