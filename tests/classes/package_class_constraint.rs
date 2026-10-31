//! IEEE 1800-2017 §8.23 / §18.3 / §26.3: imported package classes
//! remain valid roots of scoped constants inside module class constraints.
use xezim::simulate;

#[test]
fn imported_package_class_constant_constrains_the_value() {
    for import in [
        "import protocol_pkg::*;",
        "import protocol_pkg::message_codes;",
    ] {
        let src = format!(
            r#"package protocol_pkg;
  class message_codes; localparam int REQUEST = 7; endclass
endpackage
module top;
  {import}
  class request;
    rand int code;
    constraint legal_code {{ code == message_codes::REQUEST; }}
  endclass
  initial begin
    request r = new();
    repeat (3) begin
      if (!r.randomize()) $fatal(1, "randomization failed");
      $display("code %0d", r.code);
    end
    $finish;
  end
endmodule"#
        );
        let sim = simulate(&src, 10).expect("class constant resolves through the import");
        let out: Vec<_> = sim
            .output
            .iter()
            .map(|line| line.message.as_str())
            .collect();
        assert_eq!(out, ["code 7", "code 7", "code 7"], "{import}");
    }
}
