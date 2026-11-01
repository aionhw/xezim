//! A subroutine formal of a package struct type keeps its own package's
//! member layout across repeated calls.
//!
//! The layout of a named formal type is computed once and reused for later
//! calls. Two packages here declare a `cfg_t` of different shapes whose
//! member widths come from each package's own `W`, the top declares a third
//! `W`, and continuous assigns call each package's functions again whenever
//! the struct changes. A block-local typedef between the two checks moves the
//! run-time type tables, after which the layouts must come out the same.

use xezim::simulate;

fn notes(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("NOTE:"))
        .collect()
}

#[test]
fn same_named_package_struct_formals_keep_their_own_layouts() {
    let src = r#"
package pa;
  localparam int W = 3;
  typedef struct packed { logic [W-1:0] hi; logic [1:0] lo; } cfg_t;
  function automatic logic [2:0] hi_of(cfg_t c); return c.hi; endfunction
  function automatic logic [1:0] lo_of(pa::cfg_t c); return c.lo; endfunction
endpackage
package pb;
  localparam int W = 2;
  typedef struct packed { logic [1:0] lo; logic [W-1:0] hi; logic en; } cfg_t;
  function automatic logic [1:0] hi_of(cfg_t c); return c.hi; endfunction
  function automatic logic en_of(pb::cfg_t c); return c.en; endfunction
endpackage
module top;
  localparam int W = 6;
  pa::cfg_t a;
  pb::cfg_t b;
  logic [2:0] ah;
  logic [1:0] al, bh;
  logic be;
  assign ah = pa::hi_of(a);
  assign al = pa::lo_of(a);
  assign bh = pb::hi_of(b);
  assign be = pb::en_of(b);
  initial begin
    a = 5'b101_10; b = 5'b01_10_1;
    #1 $display("NOTE: %b %b %b %b", ah, al, bh, be);
    begin
      typedef logic [7:0] byte_t;
      byte_t q;
      q = 8'h5a;
    end
    a = 5'b011_01; b = 5'b10_01_0;
    #1 $display("NOTE: %b %b %b %b", ah, al, bh, be);
    $finish;
  end
endmodule
"#;
    assert_eq!(notes(src), vec!["NOTE: 101 10 10 1", "NOTE: 011 01 01 0"]);
}
