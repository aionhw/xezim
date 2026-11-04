//! x from never-reset registers (IEEE 1800-2017 §6.8 Table 6-7) and how it
//! propagates — through operators (§11.4 Table 11-1 etc.), selects with an x
//! index (§11.5.1), `?:` (§11.4.11), `case`/`casez`/`casex` (§12.5), into
//! 2-state sinks (§6.11.2, x/z become 0) and on a clock (§9.4.2) — and the
//! assertions that see it: an immediate `assert` treats x as false
//! (§16.3), concurrent assertions sample it (§16.5), `$isunknown`,
//! `$countbits`, `$onehot` (§20.9), `assume`/`cover`, severity tasks. Every
//! expected line is the reference simulator's output, printed by the design
//! itself so no simulator message format is compared.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use std::path::PathBuf;
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_x_prop_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run `src` with top `top` and extra flags: its `T|` lines and exit code.
fn run_with(tag: &str, src: &str, top: &str, flags: &[&str]) -> (Vec<String>, i32) {
    let d = scratch(tag);
    std::fs::write(d.join("tb.sv"), src).unwrap();
    let out = Command::new(xezim())
        .current_dir(&d)
        .args(flags)
        .args(["-s", top, "tb.sv"])
        .env_remove("XEZIM_INIT_REG")
        .env_remove("XEZIM_INIT_ZERO")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&d);
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    (lines, out.status.code().unwrap_or(-1))
}

fn run(tag: &str, src: &str, top: &str) -> Vec<String> {
    let (lines, code) = run_with(tag, src, top, &[]);
    assert_eq!(code, 0, "{tag}: exit code");
    lines
}

/// An uninitialised flop's x through bitwise, reduction, arithmetic and
/// relational operators, `==` against `===`, the `?:` merge, `case` (x
/// matches no item: default), `casez`/`casex` (x is a wildcard in casex
/// only), `unique case` (no match), `if (x)` (else branch), a shift by an x
/// amount, a part select and an array index with an x index — then the same
/// logic after the flop loads.
#[test]
fn uninitialised_flop_x_through_comb_logic() {
    let got = run(
        "comb",
        r#"
module xp_x1;
  logic clk = 0;
  logic [3:0] q, d = 4'b1010;   // q: flop never reset -> x
  logic       qb;
  logic [3:0] k = 4'b0110;
  logic [1:0] sel;
  always_ff @(posedge clk) q <= d;
  always_ff @(posedge clk) qb <= 1'b1;
  always_ff @(posedge clk) sel <= 2'b01;
  logic [7:0] mem [4] = '{8'h11, 8'h22, 8'h33, 8'h44};
  logic [3:0] y_and, y_or, y_xor, y_not, y_add, y_mux, y_mux2, y_shl, y_shr, y_ps;
  logic       r_and, r_or, r_xor, r_nand, c_eq, c_ceq, c_lt, c_ne, c_cne, y_if;
  logic [3:0] y_case, y_casez, y_casex, y_uc;
  logic [7:0] y_mem;
  always_comb begin
    y_and = q & 4'b0000; y_or = q | 4'b1111; y_xor = q ^ k; y_not = ~q;
    r_and = &q; r_or = |q; r_xor = ^q; r_nand = ~&q;
    y_add = q + 4'd1;
    c_eq = (q == 4'b1010); c_ceq = (q === 4'bxxxx); c_lt = (q < 4'd3); c_ne = (q != k); c_cne = (q !== k);
    y_mux = qb ? 4'b1100 : 4'b1010;
    y_mux2 = qb ? 4'b1100 : 4'b1100;
    if (qb) y_if = 1'b1; else y_if = 1'b0;
    case (sel) 2'b00: y_case = 4'h1; 2'b01: y_case = 4'h2; default: y_case = 4'hf; endcase
    casez (sel) 2'b0?: y_casez = 4'h1; default: y_casez = 4'he; endcase
    casex (sel) 2'b0?: y_casex = 4'h1; default: y_casex = 4'he; endcase
    unique case (sel) 2'b00: y_uc = 4'h1; 2'b01: y_uc = 4'h2; 2'b10: y_uc = 4'h3; 2'b11: y_uc = 4'h4; endcase
    y_shl = k << sel; y_shr = k >> q;
    y_ps = k[sel +: 2];
    y_mem = mem[sel];
  end
  initial begin
    #1 $display("T|pre  and=%b or=%b xor=%b not=%b add=%b", y_and, y_or, y_xor, y_not, y_add);
    $display("T|pre  rand=%b ror=%b rxor=%b rnand=%b", r_and, r_or, r_xor, r_nand);
    $display("T|pre  eq=%b ceq=%b lt=%b ne=%b cne=%b", c_eq, c_ceq, c_lt, c_ne, c_cne);
    $display("T|pre  mux=%b mux2=%b if=%b", y_mux, y_mux2, y_if);
    $display("T|pre  case=%h casez=%h casex=%h uc=%h", y_case, y_casez, y_casex, y_uc);
    $display("T|pre  shl=%b shr=%b ps=%b mem=%h", y_shl, y_shr, y_ps, y_mem);
    clk = 1; #1;
    $display("T|post and=%b or=%b xor=%b not=%b add=%b", y_and, y_or, y_xor, y_not, y_add);
    $display("T|post rand=%b ror=%b rxor=%b rnand=%b", r_and, r_or, r_xor, r_nand);
    $display("T|post eq=%b ceq=%b lt=%b ne=%b cne=%b", c_eq, c_ceq, c_lt, c_ne, c_cne);
    $display("T|post mux=%b mux2=%b if=%b", y_mux, y_mux2, y_if);
    $display("T|post case=%h casez=%h casex=%h uc=%h", y_case, y_casez, y_casex, y_uc);
    $display("T|post shl=%b shr=%b ps=%b mem=%h", y_shl, y_shr, y_ps, y_mem);
  end
endmodule
"#,
        "xp_x1",
    );
    let want = [
        "T|pre  and=0000 or=1111 xor=xxxx not=xxxx add=xxxx",
        "T|pre  rand=x ror=x rxor=x rnand=x",
        "T|pre  eq=x ceq=1 lt=x ne=x cne=1",
        "T|pre  mux=1xx0 mux2=1100 if=0",
        "T|pre  case=f casez=e casex=1 uc=x",
        "T|pre  shl=xxxx shr=xxxx ps=00xx mem=xx",
        "T|post and=0000 or=1111 xor=1100 not=0101 add=1011",
        "T|post rand=0 ror=1 rxor=0 rnand=1",
        "T|post eq=1 ceq=0 lt=0 ne=1 cne=1",
        "T|post mux=1100 mux2=1100 if=1",
        "T|post case=2 casez=1 casex=1 uc=2",
        "T|post shl=1100 shr=0000 ps=0011 mem=22",
    ];
    assert_eq!(got, want);
}

/// Operator corner cases on mixed 0/1/x/z operands: a controlling operand
/// masks x (`x & 0`, `x | 1`, `x && 0`), arithmetic on any x bit is all x.
#[test]
fn operators_on_partially_unknown_values() {
    let got = run(
        "ops",
        r#"
module xp_x1b;
  logic [3:0] q;        // uninitialised: x
  logic [3:0] p = 4'b1x0z;
  logic       qb;
  logic [3:0] k = 4'b0110;
  initial begin
    #1;
    $display("T|ops p&0=%b p|1=%b p^k=%b ~p=%b p+1=%b p*0=%b -p=%b", p & 4'b0, p | 4'hf, p ^ k, ~p, p + 1'b1, p * 4'd0, -p);
    $display("T|red &p=%b |p=%b ^p=%b ~|p=%b &4'b0x11=%b |4'b0x00=%b", &p, |p, ^p, ~|p, &4'b0x11, |4'b0x00);
    $display("T|cmp p==p=%b p===p=%b p==4'b1x0z=%b p===4'b1x0z=%b p!=4'b0000=%b p>q=%b", p == p, p === p, p == 4'b1x0z, p === 4'b1x0z, p != 4'b0000, p > q);
    $display("T|log !q=%b q&&0=%b q||1=%b q&&1=%b", !q, q && 1'b0, q || 1'b1, q && 1'b1);
    $display("T|mux qb?1100:1010=%b qb?p:p=%b qb?4'b1x00:4'b1000=%b", qb ? 4'b1100 : 4'b1010, qb ? p : p, qb ? 4'b1x00 : 4'b1000);
    $display("T|sh k<<q=%b k>>q[0]=%b k<<<qb=%b 1<<p=%b", k << q, k >> q[0], k <<< qb, 4'b1 << p);
    $display("T|sel k[q]=%b k[q+:2]=%b k[qb]=%b", k[q], k[q +: 2], k[qb]);
  end
endmodule
"#,
        "xp_x1b",
    );
    let want = [
        "T|ops p&0=0000 p|1=1111 p^k=1x1x ~p=0x1x p+1=xxxx p*0=xxxx -p=xxxx",
        "T|red &p=0 |p=1 ^p=x ~|p=0 &4'b0x11=0 |4'b0x00=x",
        "T|cmp p==p=x p===p=1 p==4'b1x0z=x p===4'b1x0z=1 p!=4'b0000=1 p>q=x",
        "T|log !q=x q&&0=0 q||1=1 q&&1=x",
        "T|mux qb?1100:1010=1xx0 qb?p:p=1x0x qb?4'b1x00:4'b1000=1x00",
        "T|sh k<<q=xxxx k>>q[0]=xxxx k<<<qb=xxxx 1<<p=xxxx",
        "T|sel k[q]=x k[q+:2]=xx k[qb]=x",
    ];
    assert_eq!(got, want);
}

/// A 3-stage pipeline: x on cycle 0 and through the unreset stages until the
/// data reaches them; a synchronous reset clears every stage at the reference
/// simulator's cycle; a pipeline without reset drains its x stage by stage.
#[test]
fn pipeline_x_until_data_or_reset() {
    let got = run(
        "pipe",
        r#"
module xp_x2;
  logic clk = 0, rst = 0;
  logic [3:0] s0, s1, s2, din = 4'h5;
  always_ff @(posedge clk) if (rst) s0 <= '0; else s0 <= din;
  always_ff @(posedge clk) if (rst) s1 <= '0; else s1 <= s0 + 1;
  always_ff @(posedge clk) if (rst) s2 <= '0; else s2 <= s1 ^ 4'hf;
  logic [3:0] n0, n1, n2;   // no reset: x drains out stage by stage
  always_ff @(posedge clk) begin n0 <= din; n1 <= n0; n2 <= n1; end
  always #5 clk = ~clk;
  int cyc = 0;
  always @(posedge clk) begin
    cyc++;
    #1 $display("T|cyc=%0d rst=%b s=%b,%b,%b n=%b,%b,%b", cyc, rst, s0, s1, s2, n0, n1, n2);
  end
  initial begin
    #1 $display("T|cyc0 s=%b,%b,%b n=%b,%b,%b", s0, s1, s2, n0, n1, n2);
    #20 rst = 1;
    #20 rst = 0;
    #40 $finish;
  end
endmodule
"#,
        "xp_x2",
    );
    let want = [
        "T|cyc0 s=xxxx,xxxx,xxxx n=xxxx,xxxx,xxxx",
        "T|cyc=1 rst=0 s=0101,xxxx,xxxx n=0101,xxxx,xxxx",
        "T|cyc=2 rst=0 s=0101,0110,xxxx n=0101,0101,xxxx",
        "T|cyc=3 rst=1 s=0000,0000,0000 n=0101,0101,0101",
        "T|cyc=4 rst=1 s=0000,0000,0000 n=0101,0101,0101",
        "T|cyc=5 rst=0 s=0101,0001,1111 n=0101,0101,0101",
        "T|cyc=6 rst=0 s=0101,0110,1110 n=0101,0101,0101",
        "T|cyc=7 rst=0 s=0101,0110,1001 n=0101,0101,0101",
        "T|cyc=8 rst=0 s=0101,0110,1001 n=0101,0101,0101",
    ];
    assert_eq!(got, want);
}

/// 2-state sinks take x and z bits as 0 (§6.11.2): `int`, `bit`, `byte`,
/// `shortint`, `longint`, `int'(...)` casts, an `int` array element written
/// through a variable index (`arr[k] = arr[k] * 3 + x_bit` used to stay x),
/// a `bit` array element.
#[test]
fn two_state_sinks_take_x_as_zero() {
    let got = run(
        "two_state",
        r#"
module xp_x3;
  logic [7:0] q;       // x
  logic [7:0] p = 8'b1010_x0z1;
  int i; bit [7:0] b; byte by; shortint si; longint li; bit bb;
  int arr [4];
  bit [3:0] barr [2];
  int qq [$];
  initial begin
    #1;
    i = q; b = p; by = p; si = q; li = p; bb = q[0];
    $display("T|2st i=%0d b=%b by=%0d si=%0d li=%0d bb=%b", i, b, by, si, li, bb);
    i = q + 5; $display("T|2st i=q+5 -> %0d", i);
    arr[0] = q; arr[1] = p; arr[2] = 3; arr[2] = arr[2] * 3 + q; arr[3] = 7;
    arr[3] += p;
    $display("T|arr %0d %0d %0d %0d", arr[0], arr[1], arr[2], arr[3]);
    barr[0] = p[3:0]; barr[1] = q[3:0];
    $display("T|barr %b %b", barr[0], barr[1]);
    qq.push_back(q); qq.push_back(p); $display("T|q %p", qq);
    for (int k = 0; k < 4; k++) arr[k] = arr[k] * 3 + p[k];
    $display("T|loop %0d %0d %0d %0d", arr[0], arr[1], arr[2], arr[3]);
    i = int'(q); $display("T|cast %0d %0d", i, int'(p));
  end
endmodule
"#,
        "xp_x3",
    );
    let want = [
        "T|2st i=0 b=10100001 by=-95 si=0 li=161 bb=0",
        "T|2st i=q+5 -> 0",
        "T|arr 0 161 0 0",
        "T|barr 0001 0000",
        "T|q '{0, 161}",
        "T|loop 1 0 0 0",
        "T|cast 0 161",
    ];
    assert_eq!(got, want);
}

/// The same through nonblocking writes from an `always_ff`.
#[test]
fn two_state_array_elements_from_nonblocking_writes() {
    let got = run(
        "two_state_nba",
        r#"
module xp_x5;
  logic clk = 0;
  logic [3:0] q;            // x
  int  ai [4] = '{1, 2, 3, 4};
  bit  [3:0] ab [4];
  byte by [2];
  int k = 0;
  always_ff @(posedge clk) begin ai[k] <= ai[k] + q; ab[k] <= q; by[k[0]] <= {q, q}; end
  initial begin
    #1 clk = 1; #1 clk = 0; k = 1; #1 clk = 1; #1;
    $display("T|nba ai=%p ab=%p by=%p", ai, ab, by);
  end
endmodule
"#,
        "xp_x5",
    );
    let want = ["T|nba ai='{0, 0, 3, 4} ab='{'h0, 'h0, 'h0, 'h0} by='{0, 0}"];
    assert_eq!(got, want);
}

/// A fixed unpacked array declared in a procedural block holds values of its
/// element type: `logic [1:0] s[4] = '{0,1,2,3}` stores 2-bit unsigned
/// elements (they kept the items' signed 32-bit `int` values, so `s[2]`
/// read -2 and `s[2] == 2` was false).
#[test]
fn block_local_array_elements_take_the_declared_type() {
    let got = run(
        "locals",
        r#"
module xp_locals;
  logic [1:0] m [4] = '{0,1,2,3};
  logic [1:0] m2 [4];
  initial begin
    logic [1:0] s [4] = '{0,1,2,3};
    logic [1:0] u [4];
    static logic [1:0] st [4] = '{0,1,2,3};
    automatic logic [1:0] au [4] = '{0,1,2,3};
    logic [1:0] sc = 2'd2;
    u = '{0,1,2,3};
    m2 = '{0,1,2,3};
    $display("T|m=%0d s=%0d u=%0d m2=%0d st=%0d au=%0d sc=%0d", m[2], s[2], u[2], m2[2], st[2], au[2], sc);
    $display("T|eq m=%b s=%b u=%b st=%b au=%b gt=%b", m[2]==2, s[2]==2, u[2]==2, st[2]==2, au[2]==2, s[3] > 1);
    $display("T|p s=%p", s);
  end
endmodule
"#,
        "xp_locals",
    );
    let want = [
        "T|m=2 s=2 u=2 m2=2 st=2 au=2 sc=2",
        "T|eq m=1 s=1 u=1 st=1 au=1 gt=1",
        "T|p s='{'h0, 'h1, 'h2, 'h3}",
    ];
    assert_eq!(got, want);
}

/// An x clock: 0→x and x→1 are posedges, 1→x and x→0 negedges, x→z none
/// (§9.4.2 Table 9-2); a flop clocked by it counts every posedge.
#[test]
fn x_on_a_clock_counts_edges() {
    let got = run(
        "xclk",
        r#"
module xp_x4;
  logic clk;     // starts x
  int pe, ne, ae;
  logic [3:0] cnt = 0;
  always @(posedge clk) pe++;
  always @(negedge clk) ne++;
  always @(clk) ae++;
  always_ff @(posedge clk) cnt <= cnt + 1;
  initial begin
    #1 clk = 1;     // x->1 posedge
    #1 clk = 1'bx;  // 1->x negedge
    #1 clk = 0;     // x->0 negedge
    #1 clk = 1'bx;  // 0->x posedge
    #1 clk = 1'bz;  // x->z none
    #1 clk = 1;     // z->1 posedge
    #1 clk = 1'bz;  // 1->z negedge
    #1 clk = 0;     // z->0 negedge
    #1 clk = 1;     // 0->1 posedge
    #1 $display("T|clk pe=%0d ne=%0d ae=%0d cnt=%0d", pe, ne, ae, cnt);
  end
endmodule
"#,
        "xp_x4",
    );
    let want = ["T|clk pe=4 ne=4 ae=9 cnt=4"];
    assert_eq!(got, want);
}

/// Immediate assertions on x: the expression is false, so the else branch
/// runs (§16.3); `=== 1'bx` is true; deferred (`#0`) and `final` forms too.
#[test]
fn immediate_assert_treats_x_as_false() {
    let got = run(
        "imm",
        r#"
module xp_a1;
  logic q;          // x
  logic [3:0] v;    // x
  logic [3:0] w = 4'b10x1;
  int fails = 0, passes = 0;
  initial begin
    #1;
    A1: assert (q) begin passes++; $display("T|A1 pass"); end else begin fails++; $display("T|A1 else q=%b", q); end
    A2: assert (!q) $display("T|A2 pass"); else begin fails++; $display("T|A2 else"); end
    A3: assert (q === 1'bx) begin passes++; $display("T|A3 pass"); end else $display("T|A3 else");
    A4: assert (v == 4'b0000) $display("T|A4 pass"); else begin fails++; $display("T|A4 else"); end
    A5: assert (|w) $display("T|A5 pass |w=%b", |w); else $display("T|A5 else");
    A6: assert (&w) $display("T|A6 pass"); else $display("T|A6 else &w=%b", &w);
    A7: assert (w[1]) $display("T|A7 pass"); else $display("T|A7 else");
    A8: assert (q || 1'b1) $display("T|A8 pass"); else $display("T|A8 else");
    A9: assert #0 (q) $display("T|A9 pass"); else $display("T|A9 deferred else");
    A10: assert final (v[0]) $display("T|A10 pass"); else $display("T|A10 final else");
    if (q) $display("T|if q taken"); else $display("T|if q else");
    if (!q) $display("T|if !q taken"); else $display("T|if !q else");
    #1 $display("T|imm fails=%0d passes=%0d", fails, passes);
  end
endmodule
"#,
        "xp_a1",
    );
    let want = [
        "T|A1 else q=x",
        "T|A2 else",
        "T|A3 pass",
        "T|A4 else",
        "T|A5 pass |w=1",
        "T|A6 else &w=0",
        "T|A7 else",
        "T|A8 pass",
        "T|if q else",
        "T|if !q else",
        "T|A9 deferred else",
        "T|A10 final else",
        "T|imm fails=3 passes=1",
    ];
    assert_eq!(got, want);
}

/// `assert property (@(posedge clk) disable iff (rst) !$isunknown(q))`
/// fails while the counter is x, is disabled during reset and passes after.
#[test]
fn concurrent_isunknown_fails_until_reset() {
    let got = run(
        "conc",
        r#"
module xp_a2;
  logic clk = 0, rst = 0;
  logic [3:0] q;
  always #5 clk = ~clk;
  always_ff @(posedge clk) if (rst) q <= 4'h0; else q <= q + 1;
  int pass_n = 0, fail_n = 0;
  P1: assert property (@(posedge clk) disable iff (rst) !$isunknown(q))
      begin pass_n++; $display("T|P1 pass t=%0t q=%b", $time, q); end
      else begin fail_n++; $display("T|P1 fail t=%0t q=%b", $time, q); end
  initial begin
    #22 rst = 1;
    #20 rst = 0;
    #40 $display("T|P1 pass_n=%0d fail_n=%0d", pass_n, fail_n);
    $finish;
  end
endmodule
"#,
        "xp_a2",
    );
    let want = [
        "T|P1 fail t=5 q=xxxx",
        "T|P1 fail t=15 q=xxxx",
        "T|P1 pass t=45 q=0001",
        "T|P1 pass t=55 q=0010",
        "T|P1 pass t=65 q=0011",
        "T|P1 pass t=75 q=0100",
        "T|P1 pass_n=4 fail_n=2",
    ];
    assert_eq!(got, want);
}

/// `$isunknown`, `$countbits`, `$countones`, `$onehot`, `$onehot0` on x and
/// z values. (`$countbits` with a single `'x` or `'z` control bit is left
/// out: the reference simulator returns 0 for it even on an all-x operand,
/// against §20.9; xezim counts the bits.)
#[test]
fn bit_query_functions_on_unknown_values() {
    let got = run(
        "bitq",
        r#"
module xp_a3;
  logic [7:0] q;                // x
  logic [7:0] p = 8'b0001_0x00;
  logic [7:0] z = 8'bzzzz_0001;
  logic [3:0] oh = 4'b0100;
  initial begin
    #1;
    $display("T|isunk q=%b p=%b z=%b oh=%b q0=%b", $isunknown(q), $isunknown(p), $isunknown(z), $isunknown(oh), $isunknown(q[0]));
    $display("T|cnt xz(z)=%0d xz(q)=%0d xz(p)=%0d 1(p)=%0d 0(p)=%0d 01(p)=%0d 1(z)=%0d", $countbits(z, 'x, 'z), $countbits(q, 'x, 'z), $countbits(p, 'x, 'z), $countbits(p, '1), $countbits(p, '0), $countbits(p, '0, '1), $countbits(z, '1));
    $display("T|ones q=%0d p=%0d z=%0d", $countones(q), $countones(p), $countones(z));
    $display("T|oh q=%b p=%b z=%b oh=%b | oh0 q=%b p=%b z=%b", $onehot(q), $onehot(p), $onehot(z), $onehot(oh), $onehot0(q), $onehot0(p), $onehot0(z));
    if ($isunknown(q)) $display("T|branch isunknown");
    if ($onehot(p)) $display("T|branch onehot p"); else $display("T|branch not onehot p");
  end
endmodule
"#,
        "xp_a3",
    );
    let want = [
        "T|isunk q=1 p=1 z=1 oh=0 q0=1",
        "T|cnt xz(z)=4 xz(q)=8 xz(p)=1 1(p)=1 0(p)=6 01(p)=7 1(z)=1",
        "T|ones q=0 p=1 z=1",
        "T|oh q=0 p=1 z=1 oh=1 | oh0 q=1 p=1 z=1",
        "T|branch isunknown",
        "T|branch onehot p",
    ];
    assert_eq!(got, want);
}

/// Concurrent assertions whose sampled values are x: an x antecedent does
/// not start the implication, an x consequent fails, `$rose` on x→1 is true,
/// `$stable` across x. Assertions failing in the same time step run their
/// action blocks in an unspecified order, so the lines are compared sorted.
#[test]
fn concurrent_assertions_sample_x() {
    let mut got = run(
        "sampled",
        r#"
module xp_a4;
  logic clk = 0;
  logic a, b;
  always #5 clk = ~clk;
  int ap = 0, af = 0, bp = 0, bf = 0, rp = 0, rf = 0, sp = 0, sf = 0;
  PA: assert property (@(posedge clk) a |-> b) begin ap++; $display("T|PA pass t=%0t a=%b b=%b", $time, a, b); end
      else begin af++; $display("T|PA fail t=%0t a=%b b=%b", $time, a, b); end
  PB: assert property (@(posedge clk) b) bp++; else begin bf++; $display("T|PB fail t=%0t b=%b", $time, b); end
  PR: assert property (@(posedge clk) $rose(a) |-> 1'b0) rp++; else begin rf++; $display("T|PR rose t=%0t a=%b", $time, a); end
  PS: assert property (@(posedge clk) a |=> $stable(b)) sp++; else begin sf++; $display("T|PS fail t=%0t b=%b", $time, b); end
  initial begin
    // cycle edges at 5,15,25,...
    #12 a = 1;            // x before first edge
    #10 b = 1;
    #10 a = 1'bx;
    #10 b = 1'bx; a = 1;
    #10 b = 0; a = 0;
    #10 a = 1; b = 1'bz;
    #10 a = 0; b = 1;
    #10 $display("T|sum ap=%0d af=%0d bp=%0d bf=%0d rp=%0d rf=%0d sp=%0d sf=%0d", ap, af, bp, bf, rp, rf, sp, sf);
    $finish;
  end
endmodule
"#,
        "xp_a4",
    );
    let mut want = [
        "T|PB fail t=5 b=x",
        "T|PR rose t=15 a=1",
        "T|PA fail t=15 a=1 b=x",
        "T|PB fail t=15 b=x",
        "T|PA pass t=25 a=1 b=1",
        "T|PS fail t=25 b=1",
        "T|PR rose t=45 a=1",
        "T|PA fail t=45 a=1 b=x",
        "T|PB fail t=45 b=x",
        "T|PS fail t=55 b=0",
        "T|PB fail t=55 b=0",
        "T|PR rose t=65 a=1",
        "T|PA fail t=65 a=1 b=z",
        "T|PB fail t=65 b=z",
        "T|PS fail t=75 b=1",
        "T|sum ap=1 af=3 bp=3 bf=5 rp=0 rf=3 sp=1 sf=3",
    ];
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

/// `cover property` on x (no hit; `$isunknown` hits), `assume property`
/// failing on x, immediate `cover` and `assume`.
#[test]
fn assume_and_cover_on_x() {
    let mut got = run(
        "cover",
        r#"
module xp_a5;
  logic clk = 0;
  logic a, b;
  always #5 clk = ~clk;
  int cv = 0, cvx = 0, asf = 0, asp = 0, icv = 0;
  CV:  cover property (@(posedge clk) a) begin cv++; $display("T|CV hit t=%0t a=%b", $time, a); end
  CVX: cover property (@(posedge clk) $isunknown(a)) begin cvx++; $display("T|CVX hit t=%0t", $time); end
  AS:  assume property (@(posedge clk) b) begin asp++; end else begin asf++; $display("T|AS fail t=%0t b=%b", $time, b); end
  initial begin
    #12 a = 1; b = 1;
    #10 a = 1'bx;
    #10 a = 0; b = 1'bx;
    #10 a = 1'bz; b = 0;
    #10 a = 1;
    #2;
    IC: cover (a) icv++;
    ICX: cover (b === 1'bx) icv += 10;
    IA: assume (b) $display("T|IA pass"); else $display("T|IA fail b=%b", b);
    b = 1'bx;
    IA2: assume (b) $display("T|IA2 pass"); else $display("T|IA2 fail b=%b", b);
    #5 $display("T|sum cv=%0d cvx=%0d asp=%0d asf=%0d icv=%0d", cv, cvx, asp, asf, icv);
    $finish;
  end
endmodule
"#,
        "xp_a5",
    );
    let mut want = [
        "T|CVX hit t=5",
        "T|AS fail t=5 b=x",
        "T|CV hit t=15 a=1",
        "T|CVX hit t=25",
        "T|AS fail t=35 b=x",
        "T|CVX hit t=45",
        "T|AS fail t=45 b=0",
        "T|IA fail b=0",
        "T|IA2 fail b=x",
        "T|CV hit t=55 a=1",
        "T|AS fail t=55 b=x",
        "T|sum cv=2 cvx=3 asp=2 asf=4 icv=1",
    ];
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

/// `$error`/`$warning` in assertion else branches: simulation continues and
/// the design counts them; the exit code is 0, or 1 with `--error-exit`
/// (`tests/misc/exit_codes.rs` contract).
#[test]
fn severity_tasks_in_assertion_else_branches() {
    let src = r#"
module xp_a6;
  logic q;
  int nerr = 0;
  initial begin
    #1;
    E1: assert (q) else begin nerr++; $error("q is unknown: %b", q); end
    E2: assert (q == 1'b0) else begin nerr++; $warning("q compare unknown"); end
    E3: assert (q !== 1'bx) else begin nerr++; $error("q still x"); end
    q = 1;
    E4: assert (q) else begin nerr++; $error("never"); end
    $display("T|nerr=%0d", nerr);
  end
endmodule
"#;
    let want = ["T|nerr=3"];
    let (got, code) = run_with("sev", src, "xp_a6", &[]);
    assert_eq!((got, code), (want.map(String::from).to_vec(), 0));
    let (got, code) = run_with("sev_exit", src, "xp_a6", &["--error-exit"]);
    assert_eq!((got, code), (want.map(String::from).to_vec(), 1));
}
