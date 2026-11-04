//! §9.2.2.2.1 (#844): exclusion keeps disjoint packed selections while
//! reads of a block's own written region do not make it trigger itself.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    simulate(src, 30)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

#[test]
fn other_packed_element_remains_sensitive() {
    for kind in ["always_comb", "always @*"] {
        let src = r#"
module top;
  logic [7:0] a;
  logic [1:0][7:0] ds;
  logic [7:0] sb;
  assign ds[0] = a;
  @KIND@ begin sb = ds[0] ^ 8'h01; ds[1] = sb; end
  initial begin
    a = 8'h01; #1 $display("ds1=%h", ds[1]);
    a = 8'h22; #1 $display("ds1=%h", ds[1]);
    a = 8'hxx; #1 $display("ds1=%h", ds[1]);
    a = 8'hzz; #1 $display("ds1=%h", ds[1]);
    a = 8'h44; #1 $display("ds1=%h", ds[1]);
    $finish;
  end
endmodule
"#
        .replace("@KIND@", kind);
        assert_eq!(
            messages(&src),
            ["ds1=00", "ds1=23", "ds1=xx", "ds1=xx", "ds1=45"],
            "{kind}"
        );
    }
}

#[test]
fn ascending_nested_packed_elements_in_instances() {
    let src = r#"
module leaf(input logic [7:0] a);
  localparam int L = 2;
  logic [0:1][L:L+1][7:0] ds;
  assign ds[0][L] = a;
  always_comb ds[1][L+1] = ds[0][L] ^ 8'h01;
endmodule
module top;
  logic [7:0] a = 8'h01;
  leaf u(a);
  initial begin
    #1 $display("A %h", u.ds[1][3]);
    a = 8'h32;
    #1 $display("B %h", u.ds[1][3]);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(src), ["A 00", "B 33"]);
}

#[test]
fn disjoint_packed_ranges_wider_than_64_bits() {
    for (decl, read, write) in [
        ("[127:0]", "[7:0]", "[127:120]"),
        ("[0:127]", "[120:127]", "[0:7]"),
        ("[127:0]", "[0 +: 8]", "[127 -: 8]"),
    ] {
        let src = format!(
            r#"
module top;
  logic [7:0] a = 8'h01;
  logic {decl} ds;
  assign ds{read} = a;
  always_comb ds{write} = ds{read} ^ 8'h01;
  initial begin
    #1 $display("A %h", ds{write});
    a = 8'h54;
    #1 $display("B %h", ds{write});
    $finish;
  end
endmodule
"#
        );
        assert_eq!(messages(&src), ["A 00", "B 55"], "{decl} {read} {write}");
    }
}

#[test]
fn dynamic_index_reads_keep_the_index_dependency() {
    let src = r#"
module top;
  logic [2:0][7:0] ds;
  int i = 0;
  always_comb ds[2] = ds[i];
  initial begin
    ds[0] = 8'h12; ds[1] = 8'h34;
    #1 $display("A %h", ds[2]);
    i = 1;
    #1 $display("B %h", ds[2]);
    ds[1] = 8'h56;
    #1 $display("C %h", ds[2]);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(src), ["A 12", "B 34", "C 56"]);
}

#[test]
fn whole_read_retains_the_unwritten_bits() {
    let src = r#"
module top;
  logic [15:0] ds;
  logic [7:0] a = 8'h12;
  logic [15:0] mirror;
  always_comb begin ds[7:0] = a; mirror = ds; end
  initial begin
    ds[15:8] = 8'h34;
    #1 $display("A %h", mirror);
    ds[15:8] = 8'h56;
    #1 $display("B %h", mirror);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(src), ["A 3412", "B 5612"]);
}

#[test]
fn self_written_element_does_not_rearm_after_intermediate_write() {
    let src = r#"
module top;
  logic [1:0][7:0] ds;
  logic [7:0] a = 8'h12;
  logic [7:0] observed;
  int runs = 0;
  always_comb begin
    runs++;
    ds[1] = 0;
    ds[1] = a;
    observed = ds[1];
    if (runs > 4) $fatal(1, "SELF_RETRIGGER");
  end
  initial begin
    #1 $display("A %0d %h", runs, observed);
    ds[0] = 8'h77;
    #1 $display("B %0d %h", runs, observed);
    a = 8'h34;
    #1 $display("C %0d %h", runs, observed);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(src), ["A 1 12", "B 1 12", "C 2 34"]);
}

/// §9.2.2.2.1: keeping an external region of a shared packed signal must
/// not let the block's own intermediate writes re-arm it. Use 128 bits so
/// the check also covers dependencies represented by a whole-storage mask.
#[test]
fn disjoint_read_with_two_writes_does_not_rearm_itself() {
    let src = r#"
module top;
  logic [1:0][63:0] ds;
  int runs = 0;
  always_comb begin
    runs++;
    ds[1] = 0;
    ds[1] = ds[0];
    if (runs > 4) $fatal(1, "SELF_RETRIGGER");
  end
  initial begin
    ds[0] = 64'd18;
    #1 $display("A %0d %0d", runs, ds[1]);
    ds[0] = 64'd52;
    #1 $display("B %0d %0d", runs, ds[1]);
    $finish;
  end
endmodule
"#;
    let lines = messages(src);
    assert_eq!(lines.len(), 2, "{lines:?}");
    let first: Vec<_> = lines[0].split_whitespace().collect();
    let second: Vec<_> = lines[1].split_whitespace().collect();
    assert_eq!((first[0], first[2]), ("A", "18"));
    assert_eq!((second[0], second[2]), ("B", "52"));
    let before: u32 = first[1].parse().unwrap();
    let after: u32 = second[1].parse().unwrap();
    // The initial procedural write can settle before or after the time-0
    // evaluation. Once stable, exactly one external update wakes the block.
    assert!((1..=2).contains(&before));
    assert_eq!(after, before + 1);
}

/// §7.4.1/§9.2.2.2.1: port inlining can put a packed element select after
/// a range select. The range keeps its element geometry, so writing bit 0
/// does not remove the other bits of that element from sensitivity.
#[test]
fn ranged_packed_port_preserves_element_sensitivity() {
    let src = r#"
module leaf(input logic [1:0][7:0] p,
            output logic low_bit, output logic [7:0] observed);
  always_comb begin
    low_bit = 1'b0;
    observed = p[0];
  end
endmodule
module top;
  logic [6:0] a = 7'h09;
  wire [1:0][7:0] ds;
  wire [7:0] observed;
  assign ds[0][7:1] = a;
  assign ds[1] = 0;
  leaf u(.p(ds[1:0]), .low_bit(ds[0][0]), .observed(observed));
  initial begin
    #1 $display("A %h", observed);
    a = 7'h1a;
    #1 $display("B %h", observed);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(src), ["A 12", "B 34"]);
}

/// §13.4/§23.9: a function's local writes do not write a same-named
/// module variable used by the calling always_comb's packed selection.
#[test]
fn callee_local_write_does_not_hide_callers_packed_dependency() {
    let src = r#"
module top;
  logic [7:0] a = 8'h01;
  logic [1:0][7:0] ds;
  assign ds[0] = a;
  function automatic int helper();
    int ds;
    ds = 1;
    return ds;
  endfunction
  always_comb ds[1] = ds[0] ^ helper();
  initial begin
    #1 $display("A %h", ds[1]);
    a = 8'h22;
    #1 $display("B %h", ds[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(src), ["A 00", "B 23"]);
}

/// §13.4: a nested callee's global dependency survives an intervening
/// function's same-named formal; each function has its own lexical scope.
#[test]
fn nested_callee_read_survives_callers_same_named_formal() {
    let src = r#"
module top;
  logic [7:0] a = 8'h01;
  logic [1:0][7:0] ds;
  logic hidden = 0;
  assign ds[0] = a;
  function automatic logic external_value();
    return hidden;
  endfunction
  function automatic logic relay(logic hidden);
    return external_value();
  endfunction
  always_comb ds[1] = ds[0] ^ relay(1'b0);
  initial begin
    #1 $display("A %h", ds[1]);
    a = 8'h22;
    #1 $display("B %h", ds[1]);
    hidden = 1;
    #1 $display("C %h", ds[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(src), ["A 01", "B 22", "C 23"]);
}
