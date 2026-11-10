//! #984 (A, B, D, E, G): §7.2, §7.4.1: a packed struct member that is a
//! packed array of a typedef element (`t3_t [1:0][3:0][7:0] f0`), or whose
//! dimensions live in the typedef (`t3d_t f0`), selects an element at every
//! index. Only the first index of `s.f0[i][j]` did: every later one was a
//! one-bit select of the row, so element writes dropped all but bit 0,
//! element reads kept bit 0 (an x struct read `00x`) and `$bits(s.f0[1][1])`
//! was 1. The same held for a member of a function or task formal, of a
//! nested struct, on the continuous, `always` and `initial` paths. Expected lines are the
//! reference simulator's.

use xezim::simulate;

#[test]
fn typedef_element_member_selects_every_dimension() {
    let src = r#"
typedef logic [2:0] t3_t;
typedef logic [3:0][7:0][2:0] t3d_t;
typedef t3d_t t3d2_t;
typedef struct packed { logic [15:0] pad; t3_t [3:0][7:0] f0; t3_t [3:0][7:0] f1; } s2_t;
typedef struct packed { t3_t [1:0][3:0][7:0] f0; } d_t;
typedef struct packed { t3d_t f0; } e_t;
typedef struct packed { t3d2_t f0; } e2_t;
typedef struct packed { d_t in; logic [3:0] t; } n_t;
function automatic logic [2:0] get_d(input d_t a, input int i, j, k); return a.f0[i][j][k]; endfunction
function automatic d_t set_d(input d_t a, input int i, j, k);
  d_t c; c = a; c.f0[i][j][k] = 3'd5; return c;
endfunction
task automatic get_d_task(input d_t a, input int i, j, k, output logic [2:0] r); r = a.f0[i][j][k]; endtask
module top;
  s2_t s; d_t d; e_t e; e2_t e2; n_t n;
  logic [1:0] ir, ic;
  wire [2:0] w_el = s.f0[ir][ic];
  wire [23:0] w_row = s.f0[ir];
  logic [2:0] p_el, g1, g3; d_t g2;
  always @(ir, ic) p_el = s.f0[ir][ic];
  initial begin
    #1 $display("AB x: el=%b row=%h p=%b", w_el, w_row, p_el);
    s = '0; s.f0[1][1] = 3'd2; s.f1[2][3] = 3'd5;
    s.f0[0][0] = 3'd1; s.f0[0][1] = 3'd2; s.f0[0][2] = 3'd4; s.f0[0][3] = 3'd7;
    ir = 1; ic = 1;
    #1 $display("AB: row1=%h f1row2=%h row0=%h el=%0d c=%0d w=%0d p=%0d", s.f0[1], s.f1[2], s.f0[0], s.f0[1][1], s.f0[ir][ic], w_el, p_el);
    ir = 2'bx;
    #1 $display("AB xidx: w=%b", w_el);
    d = '0; d.f0[1][1][1] = 3'd5;
    $display("D: bits=%0d %0d %0d %0d v=%0d row=%h", $bits(d.f0), $bits(d.f0[1]), $bits(d.f0[1][1]), $bits(d.f0[1][1][1]), d.f0[1][1][1], d.f0[1][1]);
    e = '0; e.f0[1][1] = 3'd5;
    $display("E: bits=%0d %0d %0d row=%h v=%0d", $bits(e.f0), $bits(e.f0[1]), $bits(e.f0[1][1]), e.f0[1], e.f0[1][1]);
    e2 = '0; e2.f0[2][3] = 3'd6;
    $display("E2: bits=%0d %0d row=%h v=%0d", $bits(e2.f0[2]), $bits(e2.f0[2][3]), e2.f0[2], e2.f0[2][3]);
    n = '0; n.in.f0[1][0][2] = 3'd3;
    $display("N: v=%0d bits=%0d", n.in.f0[1][0][2], $bits(n.in.f0[1][0]));
    g1 = get_d(d, 1, 1, 1); g2 = set_d('0, 0, 2, 3); get_d_task(d, 1, 1, 1, g3);
    $display("G: get=%0d set=%0d task=%0d", g1, g2.f0[0][2][3], g3);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "AB x: el=xxx row=xxxxxx p=xxx",
            "AB: row1=000010 f1row2=000a00 row0=000f11 el=2 c=2 w=2 p=2",
            "AB xidx: w=xxx",
            "D: bits=192 96 24 3 v=5 row=000028",
            "E: bits=96 24 3 row=000028 v=5",
            "E2: bits=24 3 row=000c00 v=6",
            "N: v=3 bits=24",
            "G: get=5 set=5 task=5",
        ]
    );
}
