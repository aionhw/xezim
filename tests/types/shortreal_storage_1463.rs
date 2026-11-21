//! IEEE 1800-2023 §6.12 and §6.24.1, binary32 assignment and cast conversion.
#[test]
fn round_at_storage_and_cast_boundaries() {
    let sim = xezim::simulate(r#"
module top;
  typedef shortreal f32;
  typedef f32 sample;
  nettype shortreal voltage;
  real drive = 16777216.25;
  sample v;
  shortreal constant_seed = 16777216.25;
  shortreal nba;
  voltage net_value;
  assign net_value = drive;
  initial begin
    v = drive;
    nba <= drive;
    #1;
    $display("RESULT %.2f %.2f %.2f %.2f %.2f %.2f", v, constant_seed,
             nba, net_value, shortreal'(drive), drive);
  end
endmodule
"#, 10_000).unwrap();
    let line = sim.output.iter().find(|line| line.message.starts_with("RESULT")).unwrap();
    assert_eq!(line.message, "RESULT 16777216.00 16777216.00 16777216.00 16777216.00 16777216.00 16777216.25");
}
