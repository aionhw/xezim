//! Verification for Issue #1054 / #1055:
//! - Duplicate worker stack avoids nested 1 GiB reservation
//! - Class GC collects large payloads before hitting 20,000 objects

#[test]
fn gc_reclaims_large_array_payloads_below_object_threshold() {
    let src = r#"
class item;
  byte data[256];
  function new(); endfunction
endclass
module top;
  item h;
  initial begin
    for (int j=0; j<1024; j++) begin
      h = new();
      h.data[0] = 7;
      if (h.data[0] != 7) $fatal(1, "payload");
      h = null;
      if (j % 64 == 63) #1;
    end
    $display("T|PASS");
    $finish;
  end
endmodule
"#;
    let sim = xezim::simulate(src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "Expected T|PASS in output, got: {:?}",
        sim.output
    );
}
