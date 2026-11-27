//! IEEE 1800-2023 §10.3.3, §23.9 and §27.4–§27.5 (#1656, #1657).

fn output(src: &str) -> Vec<String> {
    xezim::simulate(src, 100_000)
        .unwrap()
        .output
        .iter()
        .map(|line| line.message.trim().to_owned())
        .filter(|line| line.starts_with("R "))
        .collect()
}

#[test]
fn continuous_assign_delays_use_the_bound_generate_declaration() {
    let src = r#"`timescale 1ns/1ps
module top;
  int dv = 1;
  localparam EARLY = 2;
  localparam COUNT = 9;
  wire a, b;
  wire [1:0] c;
  if (1) begin : gen_shadow
    int dv = 4;
    assign #(dv) a = 1;
  end
  if (1) begin : gen_later
    assign #(EARLY) b = 1;
    localparam EARLY = 8;
  end
  for (genvar COUNT = 0; COUNT < 2; COUNT++) begin : gen_iteration
    assign #(2 + COUNT * 3) c[COUNT] = 1;
  end
  initial begin
    #0.5 $display("R %b %b %b", a, b, c);
    #2.0 $display("R %b %b %b", a, b, c);
    #2.0 $display("R %b %b %b", a, b, c);
    #1.0 $display("R %b %b %b", a, b, c);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        output(src),
        ["R x x zz", "R x 1 z1", "R 1 1 z1", "R 1 1 11"]
    );
}

#[test]
fn variable_delay_schedules_members_offset_ranges_and_concatenation() {
    let src = r#"`timescale 1ns/1ps
module top;
  typedef struct packed {logic [3:0] hi, lo;} pair_t;
  int dv = 2;
  logic [3:0] v = 0;
  wire pair_t ps;
  wire [7:4] slice;
  wire [4:7] ascending;
  wire [1:0] a, b;
  wire [3:0] whole;
  assign ps.lo = 4'h5;
  assign #(dv) ps.hi = v;
  assign #(dv) slice[7:4] = v;
  assign #(dv) {a, b} = v;
  assign #(dv) {ascending[4:5], ascending[6:7]} = v;
  assign #(dv) whole = v;
  initial begin
    #5 v = 4'ha;
    #1 $display("R %h %h %h %h %h", ps, slice, {a,b}, ascending, whole);
    #2 $display("R %h %h %h %h %h", ps, slice, {a,b}, ascending, whole);
    $finish;
  end
endmodule
"#;
    assert_eq!(output(src), ["R 05 0 0 0 0", "R a5 a a a a"]);
}
