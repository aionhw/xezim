//! Core PR #41: (1) a struct-pattern ELEMENT of an unpacked array parameter
//! evaluated to 0 (`localparam p_t A [3] = '{'{4,2'd2}, …}`); (2) one
//! `unsigned'(e)` in a constant function made the whole function
//! unevaluable, so a child-scope `localparam` from it read 0 and a typedef
//! sized by it collapsed to one bit. Expected values from the reference
//! simulator. Both `$unit`-scope and module-scope typedefs are covered.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn struct_pattern_elements_of_array_parameter() {
    let msgs = messages(
        "typedef struct packed { bit [31:0] s; bit [1:0] x; } p_t;
typedef struct { int s; bit [1:0] x; } u_t;
module tb;
  localparam p_t LP [3] = '{ '{4,2'd2}, '{5,2'd1}, '{6,2'd3} };
  parameter  p_t PP [2] = '{ '{7,2'd1}, '{8,2'd2} };
  localparam u_t LU [2] = '{ '{9,2'd1}, '{10,2'd2} };
  localparam bit [2:0] BV [2] = '{ '{1,0,1}, '{0,1,1} };
  initial begin
    $display(\"LP=%0d %0d %0d x=%0d %0d %0d PP=%0d %0d LU=%0d %0d BV=%b %b\",
      LP[0].s, LP[1].s, LP[2].s, LP[0].x, LP[1].x, LP[2].x, PP[0].s, PP[1].s, LU[0].s, LU[1].s, BV[0], BV[1]);
    $finish;
  end
endmodule",
    );
    assert!(
        msgs.iter().any(|m| m == "LP=4 5 6 x=2 1 3 PP=7 8 LU=9 10 BV=101 011"),
        "{msgs:?}"
    );
}

#[test]
fn struct_pattern_elements_with_module_scope_typedef() {
    let msgs = messages(
        // Raw string: this case is the one in the file whose SV contains a
        // `$display("...")`, and a plain literal ends at that inner quote.
        r#"module tb;
  typedef struct packed { bit [31:0] s; bit [1:0] x; } p_t;
  typedef struct { int s; bit [1:0] x; } u_t;
  localparam p_t LP [3] = '{ '{4,2'd2}, '{5,2'd1}, '{6,2'd3} };
  localparam p_t LL [2] = '{ 34'h13, 34'h15 };
  localparam u_t LU [2] = '{ '{9,2'd1}, '{10,2'd2} };
  initial begin
    $display("LP=%0d %0d %0d LL=%0d %0d LU=%0d %0d", LP[0].s, LP[1].s, LP[2].s, LL[0].s, LL[1].s, LU[0].s, LU[1].s);
    $finish;
  end
endmodule"#,
    );
    assert!(msgs.iter().any(|m| m == "LP=4 5 6 LL=4 5 LU=9 10"), "{msgs:?}");
}

#[test]
fn sign_cast_inside_constant_function_in_child_scope() {
    let msgs = messages(
        "package p;
  function automatic integer unsigned viacall (input integer unsigned n);
    return unsigned'(n);
  endfunction
  function automatic int idx_width (input int n);
    return (n > 1) ? $clog2(n) : 1;
  endfunction
endpackage
module child ();
  localparam int C = p::viacall(7);
  typedef logic [p::idx_width(5)-1:0] idx_t;
  initial $display(\"child C=%0d idxw=%0d\", C, $bits(idx_t));
endmodule
module tb;
  localparam int T = p::viacall(7);
  child u ();
  initial begin #1; $display(\"top T=%0d\", T); $finish; end
endmodule",
    );
    for want in ["child C=7 idxw=3", "top T=7"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want}: {msgs:?}");
    }
}
