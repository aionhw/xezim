//! IEEE 1800-2023 §23.3.3: formal indices retain their declared direction.
#[test]
fn ascending_udp_port_1201() {
    let r = xezim::simulate(r#"primitive udp_buf (Q, A);
  output Q;
  input A;
  table
    0 : 0 ;
    1 : 1 ;
  endtable
endprimitive
module m(P, g0, u3);
  input [0:3] P;
  output g0, u3;
  buf (g0, P[0]);
  udp_buf (u3, P[3]);
endmodule
module tb;
  reg [0:3] r;
  wire a, b, c, d, g0, u3, vg0, vu3;
  assign {a, b, c, d} = r;
  m u_cat(.P({a, b, c, d}), .g0(g0), .u3(u3));
  m u_vec(.P(r), .g0(vg0), .u3(vu3));
  initial begin
    for (int n=0; n<16; n++) begin
      r=n;
      #1;
      if (g0 !== r[0] || u3 !== r[3] || vg0 !== r[0] || vu3 !== r[3]) $fatal(1, "FAIL port ordering");
    end
    $display("PASS port ordering");
  end
endmodule
"#, 100).expect("simulate");
    assert!(
        r.output
            .iter()
            .any(|o| o.message.contains("PASS port ordering"))
    );
    assert!(
        !r.output.iter().any(|o| o.message.contains("FAIL")),
        "{:?}",
        r.output
    );
}
