//! §6.20 + §23.3.2: a module parameter used as a port actual of an instance
//! inside that module (`leaf u (.a(VREF));`) is a constant, not a net.
//!
//! The pass that creates §6.10 implicit nets for undeclared port actuals
//! skipped the sub-module's declared names, but collected them from its body
//! items only, so a header `#(parameter real VREF = 1.25)` looked
//! undeclared: every instance got a spurious "implicit 1-bit net created"
//! warning and an unused 1-bit signal. The value bound was always correct.

#[test]
fn parameter_actual_binds_value_without_warning() {
    let dir = std::env::temp_dir().join(format!("xezim_param_port_actual_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("param_actual.sv");
    std::fs::write(
        &sv,
        r#"
module leaf(input real a, input int b, output real y); assign y = a + b; endmodule
module mid #(parameter real VREF = 1.25, parameter int N = 3) (output real y, output real z);
  localparam int M = N * 2;
  leaf u(.a(VREF), .b(N), .y(y));
  leaf v(.a(VREF), .b(M), .y(z));
endmodule
module top;
  real y, z, y2, z2;
  mid m(.y(y), .z(z));
  mid #(.VREF(2.5), .N(4)) m2(.y(y2), .z(z2));
  initial begin #1 $display("R %.2f %.2f %.2f %.2f", y, z, y2, z2); $finish; end
endmodule
"#,
    )
    .expect("write sv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_NO_CACHE", "1")
        .arg(&sv)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("R 4.25 7.25 6.50 10.50"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("implicit 1-bit net") && !stdout.contains("implicit 1-bit net"),
        "spurious implicit-net warning:\n{stderr}{stdout}"
    );
}
