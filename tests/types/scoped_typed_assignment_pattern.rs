//! IEEE 1800-2023 §10.9.2: a typed assignment pattern may name its type
//! through a package scope, `pkg::type_t'{...}`. The parser folded the bare
//! `type_t'{...}` form only, so the scoped form died on `expected RBrace`
//! (xezim-core #96, contributed by davekeeshan).

use xezim::simulate;

const PKG: &str = r#"
package dm;
  typedef struct packed {
    logic [7:0]  zero1;
    logic [3:0]  nscratch;
    logic [2:0]  zero0;
    logic        dataaccess;
    logic [3:0]  datasize;
    logic [11:0] dataaddr;
  } hartinfo_t;
  localparam logic [3:0]  DataCount = 4'h2;
  localparam logic [11:0] DataAddr  = 12'h380;
endpackage
"#;

#[test]
fn package_scoped_typed_pattern_assigns_the_struct() {
    let src = format!(
        "{PKG}
module top;
  dm::hartinfo_t v = dm::hartinfo_t'{{zero1: 8'h0, nscratch: 4'h3, zero0: 3'b0,
                                     dataaccess: 1'b0, datasize: 4'h1, dataaddr: 12'h100}};
  dm::hartinfo_t p;
  initial begin
    p = dm::hartinfo_t'{{8'h0, 4'h2, 3'b0, 1'b1, dm::DataCount, dm::DataAddr}};
    #1 $display(\"v=%h,%h p=%h,%h,%b\", v.nscratch, v.dataaddr, p.nscratch, p.dataaddr, p.dataaccess);
  end
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"v=3,100 p=2,380,1"), "{msgs:?}");
}

/// A positional pattern or an unpacked-array concatenation written as the
/// actual of an unpacked-array port supplies the port's elements (§10.9.1,
/// §10.10); each element pattern fills its packed struct.
#[test]
fn package_scoped_typed_pattern_in_a_port_concatenation() {
    let src = format!(
        "{PKG}
module c(input dm::hartinfo_t h [1]);
  initial #1 $display(\"h=%h,%h\", h[0].nscratch, h[0].dataaddr);
endmodule
module top;
  c u(.h({{dm::hartinfo_t'{{8'h0, 4'h2, 3'b0, 1'b1, dm::DataCount, dm::DataAddr}}}}));
endmodule
"
    );
    let sim = simulate(&src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"h=2,380"), "{msgs:?}");
}

#[test]
fn a_call_before_a_pattern_is_still_a_parse_error() {
    let src = "module m; int a; function int foo(int x); return x; endfunction initial a = foo(1)'{1,2}; endmodule";
    assert!(simulate(src, 10).is_err());
}
