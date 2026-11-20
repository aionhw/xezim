//! §12.7.3 / #1331: procedural `foreach` over a class property whose PACKED
//! range is written with a class VALUE parameter
//! (`class row_param #(int W = 16); bit [W-1:0] row; foreach (row[i])`).
//!
//! The member's declared type is resolved against the MODULE parameter table,
//! which has no entry for `W`: the bound never evaluated, the shape walk
//! reported no dimensions, and the loop fell through to the one-iteration
//! fallback. Everything around it worked — the same member with a literal
//! `[15:0]`, a counted `for` over it, a bound written with a class
//! `localparam`, and the member's UNPACKED dimension written with a parameter
//! (`bit [7:0] row [N]`) — so only the packed bound was affected, in both the
//! single-variable and the multi-variable form.
//!
//! The width must come from the OBJECT's specialization, not the declared
//! default: one class instantiated as `#(8)` and as `#(16)` has to run 8 and
//! 16 iterations respectively, and an unparameterized object takes the
//! default.
//!
//! Every expectation is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn foreach_over_param_range_member_uses_the_object_specialization() {
    let o = out(r#"
module top;
  class row_param #(int W = 16);
    bit [W-1:0] row;
    int         iters;
    function void fill();
      iters = 0;
      foreach (row[i]) begin iters++; row[i] = 1'b1; end
    endfunction
  endclass
  initial begin
    row_param #(16) a = new();
    row_param #(8)  b = new();
    row_param       c = new();
    a.fill();
    b.fill();
    c.fill();
    $display("P=%0d/%h %0d/%h %0d/%h",
             a.iters, a.row, b.iters, b.row, c.iters, c.row);
    foreach (a.row[i]) a.row[i] = 1'b0;
    $display("CLR=%0d/%h", a.iters, a.row);
  end
endmodule
"#);
    assert!(
        o.contains("P=16/ffff 8/ff 16/ffff"),
        "parametric packed bound sized the member loop from the wrong object:\n{o}"
    );
    assert!(
        o.contains("CLR=16/0000"),
        "the write side did not reach every bit of the parametric member:\n{o}"
    );
}

#[test]
fn foreach_over_multi_packed_param_range_binds_every_loop_variable() {
    let o = out(r#"
module top;
  class p2d #(int W = 4);
    bit [W-1:0][7:0] row;
    int              iters, first_i, last_i, last_j;
    function void fill();
      iters = 0;
      first_i = -1;
      last_i = -1;
      last_j = -1;
      foreach (row[i, j]) begin
        if (first_i < 0) first_i = i;
        last_i = i;
        last_j = j;
        iters++;
        row[i][j] = 1'b1;
      end
    endfunction
  endclass
  initial begin
    p2d #(4) d = new();
    d.fill();
    $display("M=%0d/%h first=%0d last=%0d/%0d",
             d.iters, d.row, d.first_i, d.last_i, d.last_j);
  end
endmodule
"#);
    assert!(
        o.contains("M=32/ffffffff first=3 last=0/0"),
        "multi-variable foreach over a parametric packed member wrong:\n{o}"
    );
}

#[test]
fn foreach_over_unpacked_member_of_param_packed_element() {
    let o = out(r#"
module top;
  class elem #(int W = 8);
    bit [W-1:0] row [4];
    int         iters;
    function void fill();
      iters = 0;
      foreach (row[i, j]) begin iters++; row[i][j] = 1'b1; end
    endfunction
  endclass
  initial begin
    elem #(8) e = new();
    e.fill();
    $display("E=%0d/%h/%h", e.iters, e.row[0], e.row[3]);
  end
endmodule
"#);
    assert!(
        o.contains("E=32/ff/ff"),
        "parametric element bound sized the member element loop wrong:\n{o}"
    );
}
