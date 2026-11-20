//! §7.4.1, §11.5.1: a part-select through an index into a packed array
//! counts the declared labels of the selected dimension. On
//! `output logic [0:0][31:1] f`, `f[0][31:1]` is the whole element and
//! `f[0][30:2]` is its bits 29..1; on `[0:0][0:30]` the labels run from the
//! top. §23.2.2.3 makes a port's data type part of its declaration, so a
//! port selects exactly as a variable of the same type does.
//!
//! The interpreter places nested selects of a module variable by walking its
//! declared type, but an ANSI port of plain packed type in an instantiated
//! module has no declared type on file (only its recorded dimensions), so it
//! took the generic part-select store. That path mapped the labels through
//! the element's dimension twice and read a range of a non-final dimension
//! as bits: `f[0][31:1]` wrote nothing, `f[0][30:2]` landed one bit low, an
//! ascending element mirrored the wrong way and `o[1][4:2]` on a 3-D port
//! wrote three bits. Continuous assignments, `always_comb`, NBAs, `initial`
//! blocks and net ports all reached it. The same writes to a variable of the
//! submodule or of the top module were already right and are checked as
//! controls. VeeR EH2's `eh2_exu` writes its flush path this way
//! (`exu_flush_path_final[i][31:1]`), so its fetch address was x after
//! reset. Expected values follow the LRM; Verilator agrees on every line.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("PASS") || m.starts_with("FAIL"))
        .collect()
}

const SRC: &str = r#"
module sub (
    input  logic                 clk,
    output logic [0:0][31:1]     pa,
    output logic [0:0][31:1]     pc,
    output logic [0:0][31:1]     pn,
    output logic [0:0][31:1]     pi,
    output wire  [0:0][31:1]     pw,
    output logic [0:0][0:30]     pd,
    output logic [0:0][0:30]     pe,
    output logic [1:0][31:1]     pp,
    output logic [3:0][5:0][7:0] po
);
  // Ports: the element's bits are labelled 31..1 (or 0..30), not 30..0.
  assign pa[0][31:1] = 31'h1234_5678;
  always_comb pc[0][31:1] = 31'h0abc_def0;
  always_ff @(posedge clk) pn[0][31:1] <= 31'h1357_9bdf;
  initial begin
    pi = 'x;
    pi[0][30:2] = '0;
    po = 'x;
    po[1][4:2] = 24'h333231;
  end
  assign pw[0][31:1] = 31'h2468_ace0;
  assign pd[0][2:28] = '1;
  always_ff @(posedge clk) pe[0][0:3] <= 4'b1010;
  assign pp[1][31:1] = 31'h7fff_ffff;
  assign pp[0][16:1] = 16'hbeef;
  // The same writes to variables of the module.
  logic [0:0][31:1] la, lc, ln, li;
  logic [0:0][0:30] ld;
  assign la[0][31:1] = 31'h1234_5678;
  always_comb lc[0][31:1] = 31'h0abc_def0;
  always_ff @(posedge clk) ln[0][31:1] <= 31'h1357_9bdf;
  initial begin
    li = 'x;
    li[0][30:2] = '0;
  end
  assign ld[0][2:28] = '1;
endmodule

module tb;
  logic clk = 0;
  logic [0:0][31:1] pa, pc, pn, pi;
  wire  [0:0][31:1] pw;
  logic [0:0][0:30] pd, pe;
  logic [1:0][31:1] pp;
  logic [3:0][5:0][7:0] po;
  sub u (.*);
  // And to variables of the top module.
  logic [0:0][31:1] ta, tc, tn, ti;
  logic [0:0][0:30] td;
  assign ta[0][31:1] = 31'h1234_5678;
  always_comb tc[0][31:1] = 31'h0abc_def0;
  always_ff @(posedge clk) tn[0][31:1] <= 31'h1357_9bdf;
  initial begin
    ti = 'x;
    ti[0][30:2] = '0;
  end
  assign td[0][2:28] = '1;

  task automatic chk(string name, logic [255:0] got, logic [255:0] exp);
    if (got === exp) $display("PASS %s", name);
    else $display("FAIL %s got=%h exp=%h", name, got, exp);
  endtask

  localparam logic [30:0] MID = {1'bx, 29'h0, 1'bx};
  localparam logic [30:0] ASC = {2'bxx, 27'h7ff_ffff, 2'bxx};
  initial begin
    #1 clk = 1;
    #1 clk = 0;
    #1;
    chk("port assign", pa, 31'h1234_5678);
    chk("port comb", pc, 31'h0abc_def0);
    chk("port nba", pn, 31'h1357_9bdf);
    chk("port initial", pi, MID);
    chk("port net", pw, 31'h2468_ace0);
    chk("port ascending", pd, ASC);
    chk("port ascending nba", pe, {4'b1010, 27'bx});
    chk("port two elements", pp, {31'h7fff_ffff, 15'bx, 16'hbeef});
    chk("port 3-D initial", po[1], 48'hxx333231xxxx);
    chk("local assign", u.la, 31'h1234_5678);
    chk("local comb", u.lc, 31'h0abc_def0);
    chk("local nba", u.ln, 31'h1357_9bdf);
    chk("local initial", u.li, MID);
    chk("local ascending", u.ld, ASC);
    chk("top assign", ta, 31'h1234_5678);
    chk("top comb", tc, 31'h0abc_def0);
    chk("top nba", tn, 31'h1357_9bdf);
    chk("top initial", ti, MID);
    chk("top ascending", td, ASC);
    $finish;
  end
endmodule
"#;

#[test]
fn part_select_of_offset_port_dimension_places_labels_once() {
    let got = lines(SRC);
    let expected: Vec<String> = [
        "port assign",
        "port comb",
        "port nba",
        "port initial",
        "port net",
        "port ascending",
        "port ascending nba",
        "port two elements",
        "port 3-D initial",
        "local assign",
        "local comb",
        "local nba",
        "local initial",
        "local ascending",
        "top assign",
        "top comb",
        "top nba",
        "top initial",
        "top ascending",
    ]
    .iter()
    .map(|n| format!("PASS {n}"))
    .collect();
    assert_eq!(got, expected);
}
