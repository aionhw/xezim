//! §12.7.3: procedural `foreach` over ONE ELEMENT of an unpacked-array
//! class property (`foreach (rows[1][i])` on `bit [15:0] rows[4]`) — the
//! loop variable walks the element's packed bits.
//!
//! `foreach_fixed_subarray_dims` resolved the array's base name only
//! through module-scope concepts (a local rename or an instance-scope
//! hint), never through the current `this`, so a bare member name missed
//! and the loop fell into the member arm — iterating the four ELEMENTS
//! instead of the selected element's 16 bits.
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
fn foreach_over_member_subarray_element_iterates_its_bits() {
    let o = out(r#"
module top;
  class c;
    bit [15:0] rows[4];
    bit [7:0]  sml[3];
    int n1;
    int n2;
    int jmax;
    function void fill();
      n1 = 0;
      jmax  = -1;
      foreach (rows[1][i]) begin
        n1++;
        if (i > jmax) jmax = i;
        rows[1][i] = 1'b1;
      end
    endfunction
    function void fill_small();
      n2 = 0;
      foreach (sml[2][i]) begin
        n2++;
        sml[2][i] = 1'b1;
      end
    endfunction
  endclass
  initial begin
    c o = new();
    o.fill();
    o.fill_small();
    $display("N=%0d/%0d J=%0d V=%h/%h/%h/%h S=%h/%h/%h",
             o.n1, o.n2, o.jmax, o.rows[0], o.rows[1], o.rows[2], o.rows[3],
             o.sml[0], o.sml[1], o.sml[2]);
  end
endmodule
"#);
    assert!(
        o.contains("N=16/8 J=15 V=0000/ffff/0000/0000 S=00/00/ff"),
        "member subarray element foreach wrong:\n{o}"
    );
}
