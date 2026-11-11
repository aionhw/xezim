//! IEEE 1800-2023 §8.7 (#1054): objects with large array members are
//! reclaimed before 20,000 of them exist, and the run is unchanged by it.
//! That a collection ran is asserted beside the collector
//! (`array_payloads_trigger_reclamation_below_the_allocation_count`).

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
