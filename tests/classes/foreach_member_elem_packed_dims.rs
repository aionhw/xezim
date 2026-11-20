//! §12.7.3: multi-variable `foreach` over a class property whose UNPACKED
//! dimension is followed by packed ones — `bit [7:0] row [4]` with
//! `foreach (row[i, j])` — for fixed, ascending, queue, dynamic and
//! associative storage, plus 2-D-unpacked and multi-packed elements.
//!
//! The multi-variable member arm sized the loop from the property's
//! unpacked dimensions alone and never extended it with the element's
//! packed dimensions, so the inner variable stayed unbound (X): writes
//! landed nowhere, reads contributed 0, and the 3-D-unpacked shape ran
//! zero iterations.
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
fn foreach_member_two_vars_cover_element_packed_bits() {
    let o = out(r#"
module top;
  class fixed_arr;
    bit [7:0] row [4];
    int iters;
    int jmax;
    function void fill();
      iters = 0;
      jmax  = -1;
      foreach (row[i, j]) begin
        iters++;
        if (j > jmax) jmax = j;
        row[i][j] = 1;
      end
    endfunction
  endclass
  class asc_arr;
    bit [7:0] row [1:4];
    int iters;
    function void fill();
      iters = 0;
      foreach (row[i, j]) begin
        iters++;
        row[i][j] = 1;
      end
    endfunction
  endclass
  class queue_arr;
    bit [7:0] row [$];
    int iters;
    function void fill();
      iters = 0;
      row.delete();
      row.push_back(8'h00);
      row.push_back(8'h00);
      row.push_back(8'h00);
      row.push_back(8'h00);
      foreach (row[i, j]) begin
        iters++;
        row[i][j] = 1;
      end
    endfunction
  endclass
  class dyn_arr;
    bit [7:0] row [];
    int iters;
    function void fill();
      iters = 0;
      row = new[4];
      foreach (row[i, j]) begin
        iters++;
        row[i][j] = 1;
      end
    endfunction
  endclass
  class assoc_arr;
    bit [7:0] row [int];
    int iters;
    function void fill();
      iters = 0;
      for (int k = 0; k < 4; k++) row[k] = 8'h00;
      foreach (row[i, j]) begin
        iters++;
        row[i][j] = 1;
      end
    endfunction
  endclass
  initial begin
    fixed_arr a = new();
    asc_arr   b = new();
    queue_arr c = new();
    dyn_arr   d = new();
    assoc_arr e = new();
    a.fill(); b.fill(); c.fill(); d.fill(); e.fill();
    $display("F=%0d/%0d/%h/%h/%h/%h A=%0d/%h/%h Q=%0d/%h D=%0d/%h S=%0d/%h",
             a.iters, a.jmax, a.row[0], a.row[1], a.row[2], a.row[3],
             b.iters, b.row[1], b.row[4],
             c.iters, c.row[3], d.iters, d.row[3], e.iters, e.row[3]);
  end
endmodule
"#);
    assert!(
        o.contains("F=32/7/ff/ff/ff/ff A=32/ff/ff Q=32/ff D=32/ff S=32/ff"),
        "two-variable member foreach over packed elements wrong:\n{o}"
    );
}

#[test]
fn foreach_member_three_vars_and_multi_packed_elements() {
    let o = out(r#"
module top;
  class arr2d;
    bit [7:0] row [4][2];
    int iters;
    function void fill();
      iters = 0;
      foreach (row[i, j, k]) begin
        iters++;
        row[i][j][k] = 1;
      end
    endfunction
  endclass
  class multipack;
    bit [3:0][7:0] row [4];
    int iters;
    function void fill();
      iters = 0;
      foreach (row[i, j]) begin
        iters++;
        row[i][j] = 1;
      end
    endfunction
  endclass
  class read_side;
    bit [7:0] row [4];
    int iters;
    int ones;
    function void preset();
      for (int k = 0; k < 4; k++)
        row[k] = 8'hFF;
    endfunction
    function void count();
      iters = 0;
      ones  = 0;
      foreach (row[i, j]) begin
        iters++;
        ones += row[i][j];
      end
    endfunction
  endclass
  // controls: pure-packed members iterate their packed dimensions
  // (§12.7.3: 2 vars over [3:0][7:0] → 4×8=32 visits, each setting
  // one distinct bit → all ones; 3 vars over [1:0][3:0][7:0] → 64),
  // and a 2-D-unpacked `int` member → 4×2=8.
  class packed2d;
    bit [3:0][7:0] row;
    int iters;
    function void fill();
      iters = 0;
      foreach (row[i, j]) begin
        iters++;
        row[i][j] = 1;
      end
    endfunction
  endclass
  class packed3d;
    bit [1:0][3:0][7:0] row;
    int iters;
    function void fill();
      iters = 0;
      foreach (row[i, j, k]) begin
        iters++;
        row[i][j][k] = 1;
      end
    endfunction
  endclass
  class unpacked2d;
    int row [4][2];
    int iters;
    function void fill();
      iters = 0;
      foreach (row[i, j]) begin
        iters++;
        row[i][j] = 1;
      end
    endfunction
  endclass
  initial begin
    arr2d     a = new();
    multipack m = new();
    read_side r = new();
    packed2d  p = new();
    packed3d  q = new();
    unpacked2d u = new();
    a.fill(); m.fill(); r.preset(); r.count(); p.fill(); q.fill(); u.fill();
    $display("T=%0d/%h M=%0d/%h/%h R=%0d/%0d C=%0d/%h/%0d/%0d",
             a.iters, a.row[3][1], m.iters, m.row[0], m.row[3],
             r.iters, r.ones, p.iters, p.row, q.iters, u.iters);
  end
endmodule
"#);
    assert!(
        o.contains("T=64/ff M=16/01010101/01010101 R=32/32 C=32/ffffffff/64/8"),
        "three-variable / multi-packed member foreach wrong:\n{o}"
    );
}
