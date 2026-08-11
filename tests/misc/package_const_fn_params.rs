//! Package parameters whose initializers CALL constant functions —
//! reference-validated (customer X-storm root cause). Three failure modes
//! covered: (1) the package hoist evaluated fn-call inits to 0 before the
//! package's functions were registered, so every `p::PARAM` dimension in a
//! non-importing module resolved against 0; (2) a later re-eval arm
//! clobbered the healed value back to 0; (3) struct-member LAYOUTS of a
//! scoped `pkg::T` resolved member dims with the USING module's params, so
//! member writes/reads through the layout silently vanished.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const PKG: &str = r#"
package cfg;
  function automatic integer LOG2(input integer v);
    integer r;
    begin r = 0; while (v > 1) begin v = v / 2; r = r + 1; end LOG2 = r; end
  endfunction
  parameter integer DEPTH = 64;
  parameter integer LOG_DEPTH = LOG2(DEPTH);       // 6, arg is a sibling param
  parameter integer LOG_LIT   = LOG2(64);          // 6, literal arg
  parameter integer LOG_M6    = LOG2(DEPTH) - 6;   // 0 (the "-6 clamp" shape)
  typedef struct packed {
    logic [LOG_DEPTH-1:0]   addr;                  // 6
    logic [2*LOG_DEPTH-1:0] tag;                   // 12
    logic                   vld;                   // 1
  } req_t;                                         // 19 bits
endpackage
"#;

#[test]
fn scoped_fn_param_dimension_without_import() {
    // No import anywhere: the dims must still see LOG_DEPTH=6, not 0.
    let src = format!(
        "{PKG}
module tb;
  logic [cfg::LOG_DEPTH-1:0] a;
  logic [cfg::LOG_LIT-1:0]   b;
  initial $display(\"T|%0d %0d m6=%0d\", $bits(a), $bits(b), cfg::LOG_M6);
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|6 6 m6=0"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn scoped_struct_member_layout_without_import() {
    // Reference: r=54001 (addr at [18:13], vld at [0]). The layout bug read
    // addr back as 0 and dropped the write entirely.
    let src = format!(
        "{PKG}
module tb;
  cfg::req_t r;
  initial begin
    r = '0; r.addr = 6'h2a; r.vld = 1'b1;
    #1 $display(\"T|r=%h addr=%h vld=%b\", r, r.addr, r.vld);
  end
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|r=54001 addr=2a vld=1"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn scoped_struct_port_crosses_module_boundary() {
    // The child's port type is the scoped struct; before the fix the port
    // resolved to 5 bits (width mismatch warning) and the value truncated.
    let src = format!(
        "{PKG}
module child(input cfg::req_t req, output logic [31:0] echo);
  assign echo = {{13'h0, req}};
endmodule
module tb;
  cfg::req_t r;
  logic [31:0] e;
  child u(.req(r), .echo(e));
  initial begin
    r = '0; r.addr = 6'h2a; r.vld = 1'b1;
    #1 $display(\"T|echo=%h\", e);
  end
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|echo=00054001"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn same_name_params_stay_per_package_and_module_shadow_wins() {
    // Overlaying the owning package's params for a layout walk must not leak
    // across packages or override a module-local param OUTSIDE the walk.
    let src = r#"
package pa;
  function automatic integer L2(input integer v);
    integer r; begin r=0; while (v>1) begin v=v/2; r=r+1; end L2=r; end
  endfunction
  parameter integer W = L2(16);                              // 4
  typedef struct packed { logic [W-1:0] f; logic v; } sa_t;  // 5
endpackage
package pb;
  parameter integer W = 9;
  typedef struct packed { logic [W-1:0] f; logic v; } sb_t;  // 10
endpackage
module tb;
  parameter integer W = 2;
  pa::sa_t a;
  pb::sb_t b;
  logic [W-1:0] m;                                           // module W=2
  initial begin
    a = '0; a.f = 4'hf; b = '0; b.f = 9'h155;
    #1 $display("T|%0d %0d %0d a=%h b=%h", $bits(a), $bits(b), $bits(m), a, b);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|5 10 2 a=1e b=2aa"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn recursive_const_fn_and_lazy_conditional() {
    // The classic recursive clog2 idiom: the recursive call sits in the
    // UNTAKEN branch at the base case — eager branch substitution recursed
    // to the depth cap and every such parameter silently became 0.
    let src = r#"
package p;
  function automatic integer clog2r(input integer v);
    begin clog2r = (v <= 1) ? 0 : 1 + clog2r((v + 1) / 2); end
  endfunction
  parameter integer W = clog2r(100);   // 7
endpackage
module tb;
  logic [p::W-1:0] a;
  initial $display("T|%0d %0d", $bits(a), p::W);
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(msgs(&sim).iter().any(|m| m == "T|7 7"), "got {:?}", msgs(&sim));
}

#[test]
fn direct_scoped_fn_call_in_dimension() {
    // No parameter intermediary: the call itself sizes the range. The scoped
    // form parses as MemberAccess and the dim const-eval rejected it.
    let src = r#"
package p;
  function automatic integer LOG2(input integer v);
    integer r; begin r=0; while (v>1) begin v=v/2; r=r+1; end LOG2=r; end
  endfunction
  parameter integer N = 64;
endpackage
module tb;
  logic [p::LOG2(64)-1:0]   a;  // 6
  logic [p::LOG2(p::N)-1:0] b;  // 6
  initial $display("T|%0d %0d", $bits(a), $bits(b));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(msgs(&sim).iter().any(|m| m == "T|6 6"), "got {:?}", msgs(&sim));
}

#[test]
fn cross_package_nested_scoped_struct() {
    // p's struct embeds q's scoped struct; package processing order is a
    // hash-map accident, so widths AND member layouts must survive p
    // elaborating before q (this exact case read 35 bits and lost the
    // member write before the typedef fixpoint + per-level owner overlay).
    let src = r#"
package q;
  function automatic integer LOG2(input integer v);
    integer r; begin r=0; while (v>1) begin v=v/2; r=r+1; end LOG2=r; end
  endfunction
  parameter integer QW = LOG2(32);  // 5
  typedef struct packed { logic [QW-1:0] qa; } inner_t;
endpackage
package p;
  parameter integer PW = 3;
  typedef struct packed { q::inner_t i; logic [PW-1:0] pa; } outer_t; // 8
endpackage
module tb;
  p::outer_t o;
  initial begin
    o = '0; o.i.qa = 5'h15; o.pa = 3'h5;
    #1 $display("T|%0d %h %h %h", $bits(o), o, o.i.qa, o.pa);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|8 ad 15 5"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn same_fn_name_in_two_packages_binds_per_package() {
    // pa::f registered the bare name first; pb's param init must still call
    // pb's own f (it silently computed with pa's before the hoist fn overlay).
    let src = r#"
package pa;
  function automatic integer f(input integer v); begin f = v + 1; end endfunction
  parameter integer W = f(3);  // 4
endpackage
package pb;
  function automatic integer f(input integer v); begin f = v * 2; end endfunction
  parameter integer W = f(3);  // 6
endpackage
module tb;
  logic [pa::W-1:0] a;
  logic [pb::W-1:0] b;
  initial $display("T|%0d %0d", $bits(a), $bits(b));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(msgs(&sim).iter().any(|m| m == "T|4 6"), "got {:?}", msgs(&sim));
}

#[test]
fn scoped_param_as_fn_argument_cross_package() {
    // `LOG2(q::DEPTH)` — the scoped arg parses as MemberAccess and the
    // const-fn arg support check rejected it, silently zeroing W (this is
    // also the shape of a fn call in an instance param override).
    let src = r#"
package q;
  parameter integer DEPTH = 128;
endpackage
package p;
  function automatic integer LOG2(input integer v);
    integer r; begin r=0; while (v>1) begin v=v/2; r=r+1; end LOG2=r; end
  endfunction
  parameter integer W = LOG2(q::DEPTH);  // 7
endpackage
module ch #(parameter integer CW = 1)(output logic [CW-1:0] o);
  assign o = '1;
endmodule
module tb;
  logic [p::W-1:0] a;
  logic [15:0] ov;
  ch #(.CW(p::LOG2(q::DEPTH))) u(.o(ov[6:0]));
  initial #1 $display("T|%0d %0d %h", $bits(a), p::W, ov[6:0]);
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|7 7 7f"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn unpacked_array_of_scoped_structs_member_write() {
    // The array-of-structs decl site resolved the element typedef by bare
    // name only and computed member layouts without the owning package's
    // params — arr[2].f wrote into a 1-bit slice.
    let src = r#"
package p;
  function automatic integer LOG2(input integer v);
    integer r; begin r=0; while (v>1) begin v=v/2; r=r+1; end LOG2=r; end
  endfunction
  parameter integer W = LOG2(64);  // 6
  typedef struct packed { logic [W-1:0] f; logic v; } r_t;  // 7
endpackage
module tb;
  p::r_t arr [4];
  initial begin
    for (int i = 0; i < 4; i++) arr[i] = '0;
    arr[2].f = 6'h2b; arr[2].v = 1'b1;
    #1 $display("T|%0d %h %h %h", $bits(arr[2]), arr[2], arr[2].f, arr[3]);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|7 57 2b 00"),
        "got {:?}",
        msgs(&sim)
    );
}

#[test]
fn package_param_fixpoint_heals_declarations_not_just_values() {
    // A package parameter that references one the hoist has not reached yet
    // (forward reference here; the same shape arises for a bare cross-package
    // reference to a package that sorts later). Each hoist pass used to
    // restart with no bare names visible, so the reference missed on EVERY
    // pass, const-eval defaulted it to 0, and `X - 6` froze at -6. The
    // parameter VALUE healed later, but any declaration sized during the bad
    // window kept a 1-bit clamped width — value right, storage wrong, which
    // is exactly how this hides in a large design.
    let src = r#"
package cfg;
  parameter integer LOG_64B = LOG_SIZE - 6;
  parameter integer LOG_SIZE = 12;
endpackage
module tb;
  logic [cfg::LOG_64B:0] c;
  initial $display("T|%0d %0d", $bits(c), cfg::LOG_64B);
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|7 6"),
        "declaration must be sized from the HEALED value, got {:?}",
        msgs(&sim)
    );
}
