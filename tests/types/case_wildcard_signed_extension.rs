//! IEEE 1800-2023 §12.5–12.5.1: all items contribute to case signedness.
//! An all-signed case replicates the sign bit, including an X/Z sign bit.
//! Binary wildcard equality (§11.4.6) uses its own two operands.

use xezim::simulate;

fn notes(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("NOTE:"))
        .collect()
}

const SRC: &str = r#"
module top;
  logic signed [3:0] ss;
  logic signed [7:0] sel8;
  logic [3:0] rz, rx, rzs;
  logic wq;
  logic clk = 0;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    casez (ss)
      8'sb11111111: rz <= 4'd1;  // matches only via sign extension
      8'b00001111:  rz <= 4'd2;  // what plain zero extension would hit
      default:      rz <= 4'd15;
    endcase
    casex (ss)
      8'sb11111110: rx <= 4'd1;
      default:      rx <= 4'd15;
    endcase
    casez (sel8)
      4'sb?111: rzs <= 4'd1;     // Z sign bit replicates as wildcard fill
      default:  rzs <= 4'd15;
    endcase
    wq <= (sel8 ==? -4'sd2);
  end
  initial begin
    ss = -4'sd1; sel8 = 8'shA7;
    @(negedge clk) $display("NOTE: %0d %0d %0d %b", rz, rx, rzs, wq);
    ss = -4'sd2; sel8 = -8'sd2;
    @(negedge clk) $display("NOTE: %0d %0d %0d %b", rz, rx, rzs, wq);
    $finish;
  end
endmodule
"#;

#[test]
fn wildcard_case_uses_all_item_types() {
    assert_eq!(
        notes(SRC),
        [
            "NOTE: 2 15 1 0",  // -1: mixed casez zero-extends; A7 low bits 111 wild-match
            "NOTE: 15 1 15 1", // -2: casex matches; FE fails ?111; ==? -2 true
        ]
    );
}

#[test]
fn late_unsigned_item_changes_match_without_being_evaluated() {
    for keyword in ["case", "casez", "casex"] {
        let src = format!(
            r#"
module top;
  logic signed [3:0] sel = -1;
  int calls = 0;
  function automatic bit [7:0] late();
    calls++;
    return 8'h55;
  endfunction
  initial begin
    {keyword} (sel)
      8'shff: #1 $display("NOTE: wrong");
      8'sh0f: #1 $display("NOTE: zero %0d", calls);
      late(): #1 $display("NOTE: late");
    endcase
    {keyword} (sel)
      8'shff: $display("NOTE: signed");
      8'sh0f: $display("NOTE: wrong");
    endcase
  end
endmodule"#
        );
        assert_eq!(notes(&src), ["NOTE: zero 0", "NOTE: signed"], "{keyword}");
    }
}
