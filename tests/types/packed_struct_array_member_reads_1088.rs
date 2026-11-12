//! §7.4.1, §7.2.1: a member of an element of a packed array of packed
//! structs that is itself a struct member (`b.slots[i].dir` on
//! `hdr_t [0:2] slots`). In an ascending range element 0 is the most
//! significant element; the compiled read (a continuous assignment,
//! `always_comb`, a function) took that element as the stride origin, so
//! `slots[0]` read x and `slots[2]` read element 0, at top level, through a
//! port and in an instance, and a nested ascending array of structs
//! (`o.rows[1].tag`) read 0 (#1088). Expected values are the reference
//! simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The reporter's checks (#1088), every one of which the reference
/// simulator passes.
#[test]
fn packed_struct_array_member_reads_reporter_checks() {
    let out = lines(include_str!("packed_struct_array_member_reads_1088.sv"));
    let fails: Vec<&String> = out.iter().filter(|l| l.starts_with("FAIL")).collect();
    assert!(fails.is_empty(), "{fails:#?}");
    assert!(out.iter().any(|l| l == "TEST_PASS"), "{out:#?}");
}

#[test]
fn ascending_array_of_struct_member_reads_at_top_level() {
    assert_eq!(
        lines(
            r#"typedef struct packed { logic valid; logic [5:0] dir; logic [3:0] mode; } hdr_t;
typedef struct packed { hdr_t [0:2] slots; logic [9:0] base; } bus_pkt_t;
typedef struct packed { hdr_t [2:0] slots; logic [9:0] base; } desc_pkt_t;
typedef struct packed { hdr_t [1:3] slots; logic [9:0] base; } asc13_pkt_t;
module a1;
  bus_pkt_t b; desc_pkt_t d; asc13_pkt_t n; logic [1:0] idx;
  logic [5:0] c0, c1, c2, cv, d0, d2, n1, n3; logic [3:0] m0, ps;
  assign c0 = b.slots[2'd0].dir;
  assign c1 = b.slots[2'd1].dir;
  assign c2 = b.slots[2'd2].dir;
  assign cv = b.slots[idx].dir;
  assign m0 = b.slots[0].mode;
  assign ps = b.slots[1].dir[3:0];
  assign d0 = d.slots[0].dir;
  assign d2 = d.slots[2].dir;
  assign n1 = n.slots[1].dir;
  assign n3 = n.slots[3].dir;
  logic [5:0] q0, q2;
  always_comb begin q0 = b.slots[0].dir; q2 = b.slots[2].dir; end
  initial begin
    b.slots[0] = '{1'b1, 6'd39, 4'd7}; b.slots[1] = '{1'b0, 6'd56, 4'd11}; b.slots[2] = '{1'b1, 6'd46, 4'd5};
    d.slots[0] = '{1'b1, 6'd39, 4'd7}; d.slots[2] = '{1'b1, 6'd46, 4'd5};
    n.slots[1] = '{1'b1, 6'd11, 4'd1}; n.slots[3] = '{1'b1, 6'd33, 4'd3};
    idx = 1;
    #1;
    $display("A %0d %0d %0d %0d %0d %0d", c0, c1, c2, cv, m0, ps);
    $display("B %0d %0d %0d %0d", d0, d2, n1, n3);
    $display("C %0d %0d", q0, q2);
    $display("D %0d %0d %0d", b.slots[0].dir, b.slots[2].dir, b.slots[idx].dir);
  end
endmodule
"#
        ),
        [
            r#"A 39 56 46 56 7 8"#,
            r#"B 39 46 11 33"#,
            r#"C 39 46"#,
            r#"D 39 46 56"#,
        ]
    );
}

/// Through a port of an instance, and on an instance's own variable.
#[test]
fn ascending_array_of_struct_member_reads_through_ports() {
    assert_eq!(
        lines(
            r#"typedef struct packed { logic valid; logic [5:0] dir; logic [3:0] mode; } hdr_t;
typedef struct packed { hdr_t [0:2] slots; logic [9:0] base; } bus_pkt_t;
typedef struct packed { hdr_t [0:1][0:1] grid; logic [9:0] base; } grid_pkt_t;
module dut (input bus_pkt_t bus_i, input logic [1:0] sel_i, input grid_pkt_t g_i,
            output logic [5:0] d0, d2, dv, output logic [3:0] m1, output logic [5:0] g01, g10);
  assign d0 = bus_i.slots[2'd0].dir;
  assign d2 = bus_i.slots[2].dir;
  assign dv = bus_i.slots[sel_i].dir;
  assign m1 = bus_i.slots[1].mode;
  assign g01 = g_i.grid[0][1].dir;
  assign g10 = g_i.grid[1][0].dir;
endmodule
module holder;
  bus_pkt_t v;
  logic [5:0] h0, h2;
  assign h0 = v.slots[0].dir;
  assign h2 = v.slots[2].dir;
  function automatic logic [5:0] fdir(int i); return v.slots[i].dir; endfunction
endmodule
module a2;
  bus_pkt_t bus; grid_pkt_t g; logic [1:0] sel;
  logic [5:0] d0, d2, dv, g01, g10; logic [3:0] m1;
  dut u(.bus_i(bus), .sel_i(sel), .g_i(g), .d0(d0), .d2(d2), .dv(dv), .m1(m1), .g01(g01), .g10(g10));
  holder hh();
  initial begin
    bus.slots[0] = '{1'b1, 6'd39, 4'd7}; bus.slots[1] = '{1'b0, 6'd56, 4'd11}; bus.slots[2] = '{1'b1, 6'd46, 4'd5};
    g.grid[0][0] = '{1'b1, 6'd11, 4'd1}; g.grid[0][1] = '{1'b1, 6'd22, 4'd2};
    g.grid[1][0] = '{1'b1, 6'd33, 4'd3}; g.grid[1][1] = '{1'b1, 6'd44, 4'd4};
    hh.v = bus;
    sel = 2;
    #1;
    $display("A %0d %0d %0d %0d %0d %0d", d0, d2, dv, m1, g01, g10);
    $display("B %0d %0d %0d %0d", hh.h0, hh.h2, hh.fdir(0), hh.fdir(1));
    sel = 0;
    #1 $display("C %0d", dv);
  end
endmodule
"#
        ),
        [r#"A 39 46 46 11 22 33"#, r#"B 39 46 39 56"#, r#"C 39"#]
    );
}

/// Compiled writes, a descending range with a non-zero right bound, a
/// nested ascending array of structs and a function.
#[test]
fn array_of_struct_member_writes_and_nested_reads() {
    assert_eq!(
        lines(
            r#"typedef struct packed { logic valid; logic [5:0] dir; logic [3:0] mode; } hdr_t;
typedef struct packed { hdr_t [0:2] slots; logic [9:0] base; } bus_pkt_t;
typedef struct packed { hdr_t [3:1] slots; logic [9:0] base; } d31_pkt_t;
typedef struct packed { logic [3:0] tag; logic [1:0][0:2] lanes; } inner_t;
typedef struct packed { inner_t [0:1] rows; logic [7:0] id; } outer_t;
module a3;
  bus_pkt_t w, r; d31_pkt_t d; outer_t o;
  logic clk = 0;
  logic [5:0] d1, d3; logic [1:0] l0, l1; logic [3:0] t1;
  logic [1:0] k;
  always_comb begin
    w = '0;
    w.slots[0].dir = 6'd39;
    w.slots[1].mode = 4'd11;
    w.slots[k].valid = 1'b1;
  end
  initial r = '0;
  always @(posedge clk) begin
    r.slots[2].dir <= 6'd46;
    r.slots[k].mode <= 4'd9;
  end
  assign d1 = d.slots[1].dir;
  assign d3 = d.slots[3].dir;
  assign t1 = o.rows[1].tag;
  assign l0 = o.rows[0].lanes[1][2];
  assign l1 = o.rows[1].lanes[0][0];
  function automatic logic [5:0] fr(bus_pkt_t b, int i); return b.slots[i].dir; endfunction
  initial begin
    k = 2;
    d.slots[1] = '{1'b1, 6'd21, 4'd1}; d.slots[3] = '{1'b1, 6'd23, 4'd3};
    o = '0; o.rows[1].tag = 4'hA; o.rows[0].lanes[1][2] = 2'b10; o.rows[1].lanes[0][0] = 2'b01;
    #1 clk = 1; #1;
    $display("A %h %0d %0d %0d", w, w.slots[0].dir, w.slots[1].mode, w.slots[2].valid);
    $display("B %h %0d %0d", r, r.slots[2].dir, r.slots[2].mode);
    $display("C %0d %0d %h %b %b", d1, d3, t1, l0, l1);
    $display("D %0d %0d", fr(w, 0), fr(r, 2));
  end
endmodule
"#
        ),
        [
            r#"A 27001700000 39 11 1"#,
            r#"B 000000ba400 46 9"#,
            r#"C 21 23 a 00 01"#,
            r#"D 39 46"#,
        ]
    );
}
