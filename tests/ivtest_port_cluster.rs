//! Port / default-arg / uwire / wildcard-import behaviors recovered from the
//! ivtest port cluster. Representative cases (the ivtest sources themselves are
//! not vendored), each asserting the self-checking "PASSED" marker.

use xezim::simulate;

fn passes(src: &str) -> bool {
    match simulate(src, 10_000) {
        Ok(sim) => {
            let out: String = sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n");
            out.contains("PASSED") && !out.contains("FAILED")
        }
        Err(_) => false,
    }
}

/// §13.5.3: a subroutine port with a default expression may be omitted at the
/// call site, taking the default.
#[test]
fn subroutine_port_default_value() {
    assert!(passes(r#"
module t;
  function int add(int a, int b = 10);
    return a + b;
  endfunction
  initial begin
    if (add(5) == 15 && add(5, 2) == 7) $display("PASSED");
    else $display("FAILED got %0d %0d", add(5), add(5,2));
    #1 $finish;
  end
endmodule
"#));
}

/// §23.2.2.4: a module port with a default value used when the instantiation
/// omits that port.
#[test]
fn module_port_default_value() {
    assert!(passes(r#"
module child(input int a, input int b = 7, output int y);
  assign y = a + b;
endmodule
module t;
  int r;
  child u(.a(3), .y(r));
  initial begin #1;
    if (r == 10) $display("PASSED"); else $display("FAILED got %0d", r);
    #1 $finish;
  end
endmodule
"#));
}

/// §26.3: `import pkg::*` makes the package's symbols visible unqualified.
#[test]
fn wildcard_package_import() {
    assert!(passes(r#"
package pkg;
  localparam int K = 42;
  typedef enum { RED, GRN, BLU } col_e;
endpackage
module t;
  import pkg::*;
  col_e c;
  initial begin
    c = GRN;
    if (K == 42 && c == GRN) $display("PASSED");
    else $display("FAILED %0d %0d", K, c);
    #1 $finish;
  end
endmodule
"#));
}

/// §23.3.2: implicit `.name` port connection binds a port to a same-named net.
#[test]
fn implicit_named_port_connection() {
    assert!(passes(r#"
module child(input int a, output int y);
  assign y = a + 1;
endmodule
module t;
  int a, y;
  child u(.a, .y);
  initial begin a = 4; #1;
    if (y == 5) $display("PASSED"); else $display("FAILED got %0d", y);
    #1 $finish;
  end
endmodule
"#));
}

/// §6.7: a `uwire` is a single-driver net (one continuous assign here).
#[test]
fn uwire_single_driver() {
    assert!(passes(r#"
module t;
  uwire [7:0] w;
  logic [7:0] d;
  assign w = d;
  initial begin d = 8'hA5; #1;
    if (w == 8'hA5) $display("PASSED"); else $display("FAILED got %h", w);
    #1 $finish;
  end
endmodule
"#));
}
