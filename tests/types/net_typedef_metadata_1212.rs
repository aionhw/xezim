//! IEEE 1800 §6.18, §7.2.1: typedef nets preserve signing and packed members.
#[test]
fn typedef_nets_preserve_signing_and_members_in_instances() {
    let source = r#"
package p;
  typedef logic signed [3:0] signed_t;
  typedef signed_t alias_t;
  typedef logic [3:0] unsigned_t;
  typedef struct packed { signed_t hi; logic [3:0] lo; } pair_t;
  typedef struct { signed_t s; unsigned_t u; } record_t;
  typedef union packed { pair_t fields; logic [7:0] raw; } union_t;
endpackage
module child(input wire p::signed_t a, output wire p::signed_t b);
  p::record_t record_value;
  assign record_value.s = a;
  assign record_value.u = a;
  wire p::alias_t n = a;
  tri p::signed_t t = a;
  wire p::unsigned_t u = a;
  wire p::pair_t s;
  wire p::union_t v;
  assign s.hi = a;
  assign s.lo = 5;
  assign v = s;
  assign b = n;
  initial begin
    #1;
    $display("child=%0d,%0d,%0d,%0d,%0d,%h,%h", a,n,t,b,u,s.hi,v.fields.hi);
    if (record_value.s != -4 || record_value.u != 12) $fatal(1,"unpacked typedef member");
    if (a != -4 || n != -4 || t != -4 || b != -4 || u != 12 || s !== 8'hc5 || v.fields.hi !== 4'hc)
      $fatal(1, "typedef net metadata");
  end
endmodule
module top;
  wire p::signed_t t = -4;
  wire p::signed_t out;
  child c(t,out);
  initial begin #2; if (t != -4 || out != -4) $fatal; $display("PASS"); end
endmodule
"#;
    let out = xezim::simulate(source, 10).expect("simulate");
    assert!(
        out.output.iter().any(|s| s.message == "PASS"),
        "{:?}",
        out.output
    );
    assert!(
        out.output
            .iter()
            .any(|s| s.message == "child=-4,-4,-4,-4,12,c,c"),
        "{:?}",
        out.output
    );
}
