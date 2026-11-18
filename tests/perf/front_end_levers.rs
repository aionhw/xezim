//! Front-end (compile time and memory) levers on large flat designs, each
//! guarded by a design whose values would move if the lever changed what is
//! simulated. Every expected `T|` line was taken from the reference
//! simulator.
//!
//! - A net written bit by bit and read whole by many entries is ordered
//!   through one join node instead of writers x readers edges; the design
//!   below crosses the threshold with a ripple chain (readers that are also
//!   writers of the net) and a combinational loop through the joined net, and
//!   must simulate the same with the join disabled.
//! - Edge blocks drop their source statements after compilation, except
//!   where a later reader needs them: sampled-value calls, whole-array
//!   writes, uncompiled bodies, merged members.
//! - The per-instance parameter fixpoint skips bodies without parameter,
//!   localparam or typedef declarations; the struct passes skip designs
//!   without unpacked structs.
//! - The identity-net collapse runs on a local mirror of the name table.
//! - Sensitivity terms keep `iff` guards and expression values out of line.
//! - Instantiated `@(...)` always blocks stay lazy (shared source plus
//!   instance context) until they compile; the edge-select alias, computed
//!   edge terms, the buffer-collapse write census, `%m`, sampled-value calls
//!   and whole-array writes must all come out as with eager trees.
use std::process::Command;

fn run(name: &str, src: &str, env: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_front_end_levers_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).expect("write temporary design");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "-s",
        "tb",
        "--no-cache",
        path.to_str().unwrap(),
    ]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "run failed:\n{text}");
    text
}

fn t_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("T|")).collect()
}

const TOPO_JOIN: &str = r#"
module tb;
  localparam W = 128, R = 160;
  logic [W-1:0] a, b;
  logic [W-1:0] v;
  logic [W:0] c;
  logic [R-1:0] y;
  logic fb;
  genvar i;
  for (i = 0; i < W; i++) begin : g_w
    assign v[i] = a[i] | (fb & b[i]);
    assign c[i+1] = c[i] ^ v[i];
  end
  assign c[0] = a[0];
  assign fb = &v[3:0];
  for (i = 0; i < R; i++) begin : g_r
    assign y[i] = v[i % W] ^ v[(i * 7 + 3) % W] ^ c[W];
  end
  function automatic logic [R-1:0] expect_y(logic [W-1:0] vv, logic cw);
    for (int k = 0; k < R; k++) expect_y[k] = vv[k % W] ^ vv[(k * 7 + 3) % W] ^ cw;
  endfunction
  initial begin
    for (int t = 0; t < 4; t++) begin
      case (t)
        0: begin a = '0; b = '0; end
        1: begin a = {W/8{8'h0f}}; b = {W/8{8'h33}}; end
        2: begin a = {W/4{4'h9}}; b = {W{1'b1}}; end
        3: begin a = {64'hdead_beef_0123_4567, 64'h0f0f_0f0f_ffff_000f}; b = {W/2{2'b10}}; end
      endcase
      #1;
      $display("T|t=%0d fb=%b c=%b v=%h y=%h ok=%0d", t, fb, c[W], v, y, y === expect_y(v, c[W]));
    end
    $finish;
  end
endmodule
"#;

const EDGE_AST: &str = r#"
module leaf(input clk, input [7:0] d, output logic [7:0] q);
  always @(posedge clk) q <= d + 8'd1;
endmodule
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] d = 0;
  logic s = 0, rose_seen = 0, fell_seen = 0;
  logic [7:0] q [0:11];
  logic [7:0] mem [0:3];
  logic [7:0] acc = 0;
  genvar i;
  // Twelve same-clock NBA-only flops: merged into shared blocks, whose other
  // members never fire on their own again.
  for (i = 0; i < 12; i++) begin : g
    leaf u(.clk(clk), .d(d ^ i[7:0]), .q(q[i]));
  end
  // Sampled-value calls inside an edge block keep their clocking.
  always @(posedge clk) begin
    if ($rose(s)) rose_seen <= 1;
    if ($fell(s)) fell_seen <= 1;
  end
  // A whole-array write (the writer census reads its statement).
  always @(posedge clk) if (d == 8'd3) mem <= '{8'd1, 8'd2, 8'd3, 8'd4};
  // A body with a %m display and a blocking accumulate.
  always @(posedge clk) begin
    acc = acc + d;
    if (d == 8'd5) $display("T|m=%m acc=%0d", acc);
  end
  initial begin
    @(negedge clk);
    for (int t = 0; t < 8; t++) begin
      d = t[7:0];
      s = (t == 2 || t == 3);
      @(negedge clk);
    end
    $display("T|q0=%0d q5=%0d q11=%0d rose=%0d fell=%0d mem=%0d,%0d,%0d,%0d acc=%0d",
             q[0], q[5], q[11], rose_seen, fell_seen, mem[0], mem[1], mem[2], mem[3], acc);
    $finish;
  end
endmodule
"#;

const PARAM_FIXPOINT: &str = r#"
// No body declarations: header parameters alone drive the generate.
module nodecl #(parameter N = 3) (input [7:0] a, output [7:0] y);
  genvar k;
  for (k = 0; k < 8; k++) begin : g
    if (k < N) begin : lo
      assign y[k] = a[k];
    end else begin : hi
      assign y[k] = ~a[k];
    end
  end
endmodule
// A localparam declared inside a generate block.
module genlp #(parameter W = 4) (input [7:0] a, output [7:0] y);
  genvar k;
  for (k = 0; k < 2; k++) begin : g
    localparam SH = W * (k + 1);
    assign y[k*4 +: 4] = a[k*4 +: 4] ^ SH[3:0];
  end
endmodule
// A typedef and an enum whose values depend on a parameter.
module tdecl #(parameter B = 2) (input [7:0] a, output [7:0] y);
  typedef logic [B*2-1:0] half_t;
  typedef enum logic [7:0] { E0 = B, E1 = B + 10 } e_t;
  half_t h;
  assign h = a[B*2-1:0];
  assign y = {4'(h), 4'(E1)};
endmodule
module tb;
  logic [7:0] a = 8'hA5;
  logic [7:0] y1, y2, y3, y4, y5;
  nodecl #(.N(2)) u1(.a(a), .y(y1));
  nodecl #(.N(6)) u2(.a(a), .y(y2));
  genlp #(.W(3)) u3(.a(a), .y(y3));
  tdecl #(.B(1)) u4(.a(a), .y(y4));
  tdecl #(.B(3)) u5(.a(a), .y(y5));
  initial begin
    #1 $display("T|y1=%h y2=%h y3=%h y4=%h y5=%h", y1, y2, y3, y4, y5);
    a = 8'h3C;
    #1 $display("T|y1=%h y2=%h y3=%h y4=%h y5=%h", y1, y2, y3, y4, y5);
    $finish;
  end
endmodule
"#;

const STRUCT_PASSES: &str = r#"
package p; typedef struct { logic [7:0] a; logic [3:0] b; } s_t; endpackage
import p::*;
module ch(input logic [7:0] x, output s_t o);
  assign o.a = x + 8'd1;
  assign o.b = x[3:0];
endmodule
module tb;
  logic [7:0] x = 8'd7;
  s_t s1, s2, s3;
  ch u(.x(x), .o(s1));
  assign s2 = s1;
  assign s3 = '{x, 4'd9};
  initial begin
    #1 $display("T|a=%0d b=%0d a2=%0d b2=%0d a3=%0d b3=%0d", s1.a, s1.b, s2.a, s2.b, s3.a, s3.b);
    x = 8'd20;
    #1 $display("T|a=%0d b=%0d a2=%0d b2=%0d a3=%0d b3=%0d", s1.a, s1.b, s2.a, s2.b, s3.a, s3.b);
    $finish;
  end
endmodule
"#;

const COLLAPSE_CHAIN: &str = r#"
module leaf(input [3:0] a, output [3:0] y);
  assign y = a + 4'd1;
endmodule
module mid(input [3:0] m, output [3:0] y);
  leaf u(.a(m), .y(y));
endmodule
module top_(input [3:0] t, output [3:0] y);
  mid u(.m(t), .y(y));
endmodule
module tb;
  logic [3:0] src = 4'd3;
  wire [3:0] y;
  // A buffer chain written sink-first: collapsing it takes several passes.
  wire [3:0] b4, b3, b2, b1, b0;
  assign b4 = b3;
  assign b3 = b2;
  assign b2 = b1;
  assign b1 = b0;
  assign b0 = src;
  top_ u(.t(b4), .y(y));
  initial begin
    #1 $display("T|y=%0d b4=%0d leaf_a=%0d mid_m=%0d", y, b4, u.u.u.a, u.u.m);
    src = 4'd9;
    #1 $display("T|y=%0d b4=%0d leaf_a=%0d mid_m=%0d", y, b4, u.u.u.a, u.u.m);
    $finish;
  end
endmodule
"#;

const SENS_TERMS: &str = r#"
module tb;
  logic clk = 0; always #5 clk = ~clk;
  logic en = 0;
  logic [3:0] a = 0, b = 0;
  int n_iff = 0, n_sum = 0, n_or = 0, n_blk = 0;
  logic [3:0] mem [0:3];
  int k = 1;
  always @(posedge clk iff en) n_iff++;
  always @(posedge clk iff en) begin
    n_blk <= n_blk + 1;
  end
  covergroup cg @(posedge clk iff en);
    coverpoint a;
  endgroup
  cg c = new;
  initial forever begin
    @(a + b);
    n_sum++;
  end
  initial forever begin
    @(a or b);
    n_or++;
  end
  int n_mem = 0;
  initial forever begin
    @(mem[k]);
    n_mem++;
  end
  initial begin
    mem[0] = 0; mem[1] = 0; mem[2] = 0; mem[3] = 0;
    #12 en = 1;
    #20 en = 0;
    a = 2; b = 1;
    #1 a = 1; b = 2;
    #1 a = 3;
    #1 mem[2] = 5;
    #1 mem[1] = 7;
    #1 k = 2;
    #1 mem[2] = 6;
    #20 $display("T|iff=%0d blk=%0d sum=%0d or=%0d mem=%0d cov=%0.1f", n_iff, n_blk, n_sum, n_or, n_mem, c.get_coverage());
    $finish;
  end
endmodule
"#;

const LAZY_CHILDREN: &str = r#"
module flop(input clk, input [3:0] d, output reg [3:0] q);
  reg [3:0] r;
  always @(posedge clk) begin r <= d; q <= r; end
endmodule
module edgecnt(input ck, input x, output reg [3:0] n, output reg [3:0] rises);
  initial begin n = 0; rises = 0; end
  always @(posedge ck) n <= n + 1;
  always @(posedge ck) if ($rose(x)) rises <= rises + 1;
endmodule
module arrw(input clk, input go, output [7:0] s);
  reg [3:0] m [0:1];
  initial begin m[0] = 0; m[1] = 0; end
  always @(posedge clk) if (go) m <= '{4'd5, 4'd9};
  assign s = {m[0], m[1]};
endmodule
module named(input clk, input [3:0] d);
  reg [3:0] v;
  always @(posedge clk) begin
    v <= d;
    if (d == 4'd6) $display("T|m=%m v=%0d", v);
  end
endmodule
module tb;
  reg clk = 0; always #5 clk = ~clk;
  reg [3:0] bus = 0, d = 0;
  reg a = 0, en = 0, x = 0, go = 0;
  wire [3:0] q [0:1];
  wire [3:0] q0, q1, n_bit, r_bit, n_and, r_and;
  assign q0 = q[0];
  assign q1 = q[1];
  wire [7:0] s;
  // Clock from a vector bit, through a generate loop.
  genvar i;
  for (i = 0; i < 2; i++) begin : g
    flop u(.clk(bus[i]), .d(d ^ i[3:0]), .q(q[i]));
  end
  // A clock that is an expression of the parent's nets.
  edgecnt c1(.ck(bus[2]), .x(x), .n(n_bit), .rises(r_bit));
  edgecnt c2(.ck(a & en), .x(x), .n(n_and), .rises(r_and));
  wire [3:0] n_clk, r_clk;
  edgecnt c3(.ck(clk), .x(x), .n(n_clk), .rises(r_clk));
  arrw w(.clk(clk), .go(go), .s(s));
  named nm(.clk(clk), .d(d));
  // Whole-net buffers onto names the child blocks write.
  wire [3:0] b_r, b_q;
  assign b_r = g[0].u.r;
  assign b_q = q1;
  initial begin
    for (int t = 0; t < 12; t++) begin
      @(negedge clk);
      d = t[3:0];
      bus = bus + 1;
      a = t[0];
      en = (t > 3);
      x = (t == 2 || t == 3 || t == 7);
      go = (t == 5);
    end
    #1 $display("T|q0=%0d q1=%0d br=%0d bq=%0d nb=%0d na=%0d ra=%0d nc=%0d s=%h",
                q0, q1, b_r, b_q, n_bit, n_and, r_and, n_clk, s);
    $display("R|rc=%0d", r_clk);
    $finish;
  end
endmodule
"#;

#[test]
fn topo_join_orders_bit_driven_nets() {
    let want = [
        "T|t=0 fb=0 c=0 v=00000000000000000000000000000000 y=0000000000000000000000000000000000000000 ok=1",
        "T|t=1 fb=1 c=1 v=3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f y=0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f ok=1",
        "T|t=2 fb=1 c=1 v=ffffffffffffffffffffffffffffffff y=ffffffffffffffffffffffffffffffffffffffff ok=1",
        "T|t=3 fb=1 c=1 v=feafbeefababefefafafafafffffaaaf y=7f778a0ddc0f3e6f8b0b6f6f0f050f877f778a0d ok=1",
    ];
    let joined = run("topo_join", TOPO_JOIN, &[("XEZIM_COMPILE_PHASES", "1")]);
    assert_eq!(t_lines(&joined), want, "{joined}");
    // `v` (128 writers) and the ripple chain `c` (129 writers, 128 of which
    // also read it) both cross the writers x readers threshold.
    assert!(
        joined.contains("2 join nodes"),
        "expected both wide nets joined:\n{joined}"
    );
    let plain = run("topo_join_off", TOPO_JOIN, &[("XEZIM_TOPO_JOIN", "0")]);
    assert_eq!(t_lines(&plain), want, "{plain}");
}

#[test]
fn edge_blocks_keep_the_statements_later_readers_need() {
    let out = run("edge_ast", EDGE_AST, &[("XEZIM_VERBOSE", "1")]);
    assert_eq!(
        t_lines(&out),
        [
            "T|m=tb acc=15",
            "T|q0=8 q5=3 q11=13 rose=1 fell=1 mem=1,2,3,4 acc=28",
        ],
        "{out}"
    );
    assert!(
        out.contains("[EDGE-MERGE] merged 12 blocks"),
        "the leaf flops must merge:\n{out}"
    );
}

#[test]
fn parameter_free_bodies_still_expand_their_generates() {
    let out = run("param_fixpoint", PARAM_FIXPOINT, &[]);
    assert_eq!(
        t_lines(&out),
        [
            "T|y1=59 y2=65 y3=c6 y4=1b y5=5d",
            "T|y1=c0 y2=fc y3=5f y4=0b y5=cd",
        ],
        "{out}"
    );
}

#[test]
fn unpacked_struct_assigns_still_expand() {
    let out = run("struct_passes", STRUCT_PASSES, &[]);
    assert_eq!(
        t_lines(&out),
        [
            "T|a=8 b=7 a2=8 b2=7 a3=7 b3=9",
            "T|a=21 b=4 a2=21 b2=4 a3=20 b3=9",
        ],
        "{out}"
    );
}

#[test]
fn identity_chains_collapse_in_any_order() {
    let out = run("collapse_chain", COLLAPSE_CHAIN, &[("XEZIM_DEBUG", "1")]);
    assert_eq!(
        t_lines(&out),
        [
            "T|y=4 b4=3 leaf_a=3 mid_m=3",
            "T|y=10 b4=9 leaf_a=9 mid_m=9",
        ],
        "{out}"
    );
    // Same re-pointing work as the in-place passes did.
    assert!(
        out.contains("[NET-COLLAPSE] 2 identity port nets collapsed"),
        "{out}"
    );
    assert!(
        out.contains("[BUF-COLLAPSE] 24 identity buffer nets collapsed"),
        "{out}"
    );
}

#[test]
fn guarded_and_expression_event_terms() {
    let out = run("sens_terms", SENS_TERMS, &[]);
    assert_eq!(
        t_lines(&out),
        ["T|iff=2 blk=2 sum=2 or=3 mem=4 cov=6.2"],
        "{out}"
    );
}

#[test]
fn lazy_child_always_blocks_match_eager_trees() {
    let lazy = run("lazy_children", LAZY_CHILDREN, &[("XEZIM_DEBUG", "1")]);
    assert_eq!(
        t_lines(&lazy),
        [
            "T|m=tb.nm v=5",
            "T|q0=8 q1=4 br=10 bq=4 nb=2 na=4 ra=0 nc=12 s=59",
        ],
        "{lazy}"
    );
    let eager = run(
        "lazy_children_off",
        LAZY_CHILDREN,
        &[("XEZIM_DEBUG", "1"), ("XEZIM_LAZY_ALWAYS", "0")],
    );
    let keep = |t: &str| -> Vec<String> {
        t.lines()
            .filter(|l| l.starts_with("T|") || l.starts_with("R|") || l.contains("COLLAPSE]"))
            .map(str::to_string)
            .collect()
    };
    // Same values, and the same buffer collapses: the write census saw the
    // lazy blocks' targets exactly as it sees materialized ones.
    assert_eq!(keep(&lazy), keep(&eager), "lazy:\n{lazy}\neager:\n{eager}");
    assert!(lazy.contains("[BUF-COLLAPSE]"), "{lazy}");
    // §16.9.3: `$rose(x)` in the clk-clocked instance keeps its own history
    // although two sibling instances of the module run on other clocks (the
    // reference counts 2; a span-keyed site shared the first one's clock).
    assert!(lazy.contains("R|rc=2"), "{lazy}");
}

/// IEEE 1800-2023 §§10.11, 13.4: empty collapse work preserves explicit
/// aliases and called procedural code, including the diagnostic census.
#[test]
fn zero_continuous_assign_census_preserves_procedural_aliases() {
    let out = run(
        "empty_collapse",
        r#"
module tb;
  logic [7:0] state, result;
  wire [7:0] a, b;
  alias a = b;
  function automatic integer next_value(input integer n);
    state = n + 1;
    next_value = state;
  endfunction
  initial begin
    result = next_value(4);
    force a = 8'h5c;
    #1;
    $display("T|state=%0d result=%0d alias=%h", state, result, b);
    release a;
    $finish;
  end
endmodule
"#,
        &[("XEZIM_BUF_CENSUS", "1")],
    );
    assert_eq!(t_lines(&out), ["T|state=5 result=5 alias=5c"], "{out}");
    let census: Vec<_> = out
        .lines()
        .filter(|line| line.starts_with("[BUF-CENSUS]"))
        .collect();
    assert_eq!(
        census,
        ["[BUF-CENSUS] cont_assigns=0 identity=0 identity_single_driver=0"; 2],
        "{out}"
    );
}
