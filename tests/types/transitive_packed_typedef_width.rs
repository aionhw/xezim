/// A typedef and its enclosing packed struct use the settled package width (§6.18, §26.3).
#[test]
fn transitive_packed_typedef_width() {
    let result = xezim::simulate(
        r#"
package cfg;
  localparam AW = 4;
endpackage
package bus;
  localparam Hosts = 2;
  localparam Slv = cfg::AW;
  localparam Mst = Slv + $clog2(Hosts);
  typedef logic [Mst-1:0] id_t;
  typedef logic [Slv+$clog2(Hosts)-1:0] direct_t;
  typedef struct packed { id_t id; logic [99:0] payload; } channel_t;
endpackage
module top;
  bus::channel_t ch;
  initial $display("widths=%0d,%0d,%0d", $bits(bus::id_t), $bits(bus::direct_t), $bits(ch));
endmodule
"#,
        100,
    )
    .expect("simulate");
    assert!(
        result
            .output
            .iter()
            .any(|line| line.message == "widths=5,5,105")
    );
}
