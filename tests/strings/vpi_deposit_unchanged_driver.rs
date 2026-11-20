use std::path::{Path, PathBuf};
use std::process::Command;

const DEPOSIT_C: &str = r#"
#include "vpi_user.h"

static void deposit(const char *path, s_vpi_value *value) {
    vpiHandle net = vpi_handle_by_name((PLI_BYTE8 *)path, NULL);
    if (!net) {
        vpi_printf("Could not find %s\n", path);
        return;
    }
    vpi_put_value(net, value, NULL, vpiNoDelay);
    vpi_free_object(net);
}

void deposit_real(const char *path) {
    s_vpi_value value = { .format = vpiRealVal, .value.real = -4.5 };
    deposit(path, &value);
}

void deposit_bits(const char *path) {
    s_vpi_vecval bits = { .aval = 0x4a, .bval = 0 };
    s_vpi_value value = { .format = vpiVectorVal, .value.vector = &bits };
    deposit(path, &value);
}
"#;

const CONDITIONAL_REAL: &str = r#"
`timescale 1ns/1ps
module top;
  import "DPI-C" context function void deposit_real(input string path);
  NETTYPE_DECLARATION
  RealNet n;
  bit sel = 1;
  real a = 2.75, b = 9.0;
  assign n = sel ? a : b;

  initial begin
    #1 $display("T|initial %.2f", n);
    deposit_real("top.n");
    #1 $display("T|deposited %.2f", n);
    b = 10.0;
    #1 $display("T|unselected input changed %.2f", n);
    a = -1.25;
    #1 $display("T|driver changed %.2f", n);
    $finish;
  end
endmodule
"#;

const LOCAL_RESOLVER: &str = r#"
  function automatic real first_driver(input real drivers[]);
    return drivers[0];
  endfunction
  nettype real RealNet with first_driver;
"#;

const CONDITIONAL_WIRE: &str = r#"
`timescale 1ns/1ps
module top;
  import "DPI-C" context function void deposit_bits(input string path);
  bit sel = 1;
  logic [7:0] a = 8'h25, b = 8'h99;
  wire [7:0] n;
  assign n = sel ? a : b;

  initial begin
    #1 $display("T|initial %h", n);
    deposit_bits("top.n");
    #1 $display("T|deposited %h", n);
    b = 8'h88;
    #1 $display("T|unselected input changed %h", n);
    a = 8'h17;
    #1 $display("T|driver changed %h", n);
    $finish;
  end
endmodule
"#;

const SUM_DRIVERS: &str = r#"
`timescale 1ns/1ps
module top;
  import "DPI-C" context function void deposit_real(input string path);
  function automatic real sum(input real drivers[]);
    real total = 0.0;
    foreach (drivers[i]) total += drivers[i];
    return total;
  endfunction

  nettype real SumNet with sum;
  SumNet n;
  bit swap;
  assign n = swap ? 3.0 : 1.5;
  assign n = swap ? 1.0 : 2.5;

  initial begin
    #1 $display("T|initial %.2f", n);
    deposit_real("top.n");
    #1 $display("T|deposited %.2f", n);
    swap = 1;
    #1 $display("T|drivers changed %.2f", n);
    $finish;
  end
endmodule
"#;

const WOR_DRIVERS: &str = r#"
`timescale 1ns/1ps
module top;
  import "DPI-C" context function void deposit_bits(input string path);
  wor [7:0] n;
  logic [7:0] a = 8'h25, b = 8'h03;
  assign n = a;
  assign n = b;

  initial begin
    #1 $display("T|initial %h", n);
    deposit_bits("top.n");
    #1 $display("T|deposited %h", n);
    b = 8'h02;
    #1 $display("T|driver changed %h", n);
    $finish;
  end
endmodule
"#;

const DIRECT_COPY: &str = r#"
`timescale 1ns/1ps
module top;
  import "DPI-C" context function void deposit_bits(input string path);
  logic [7:0] source = 8'h25;
  wire [7:0] n;
  assign n = source;

  initial begin
    #1 $display("T|initial %h", n);
    deposit_bits("top.n");
    #1 $display("T|deposited %h", n);
    source = 8'h17;
    #1 $display("T|source changed %h", n);
    $finish;
  end
endmodule
"#;

const CONTINUOUS_READER: &str = r#"
`timescale 1ns/1ps
module top;
  import "DPI-C" context function void deposit_real(input string path);
  nettype real RealNet;
  RealNet n;
  real source = 2.75, copy;
  assign n = source;
  assign copy = n;

  initial begin
    #1 $display("T|initial %.2f %.2f", n, copy);
    deposit_real("top.n");
    #1 $display("T|deposited %.2f %.2f", n, copy);
    $finish;
  end
endmodule
"#;

fn scratch() -> PathBuf {
    let made = Command::new("mktemp")
        .arg("-d")
        .arg(std::env::temp_dir().join("xezim-vpi-deposit.XXXXXX"))
        .output()
        .expect("failed to launch mktemp");
    assert!(made.status.success(), "{made:?}");
    PathBuf::from(String::from_utf8(made.stdout).unwrap().trim_end())
}

fn check_deposit(sv: &str, expected: &[&str]) {
    let dir = scratch();
    std::fs::write(dir.join("deposit.c"), DEPOSIT_C).unwrap();
    std::fs::write(dir.join("top.sv"), sv).unwrap();

    let mut cc = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()));
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    let built = cc
        .arg("-I")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("include"))
        .args(["deposit.c", "-o", "deposit.so"])
        .current_dir(&dir)
        .output()
        .expect("failed to launch C compiler");
    assert!(built.status.success(), "{built:?}");

    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--no-cache",
            "--dpi-lib",
            "./deposit.so",
            "-s",
            "top",
            "top.sv",
        ])
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    std::fs::remove_dir_all(dir).unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    let values: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix("T|"))
        .collect();
    assert_eq!(values, expected, "{text}");
}

fn check_conditional_real(nettype: &str) {
    check_deposit(
        &CONDITIONAL_REAL.replace("NETTYPE_DECLARATION", nettype),
        &[
            "initial 2.75",
            "deposited -4.50",
            "unselected input changed -4.50",
            "driver changed -1.25",
        ],
    );
}

/// IEEE 1800-2023 §6.6.7, §11.4.11 and §38.34.
#[test]
fn resolved_real_deposit_survives_unselected_input_change() {
    check_conditional_real(LOCAL_RESOLVER);
}

/// IEEE 1800-2023 §6.6.7, §11.4.11 and §38.34.
#[test]
fn unresolved_real_deposit_survives_unselected_input_change() {
    check_conditional_real("nettype real RealNet;");
}

/// IEEE 1800-2023 §6.6.1, §11.4.11 and §38.34.
#[test]
fn wire_deposit_survives_unselected_input_change() {
    check_deposit(
        CONDITIONAL_WIRE,
        &[
            "initial 25",
            "deposited 4a",
            "unselected input changed 4a",
            "driver changed 17",
        ],
    );
}

/// IEEE 1800-2023 §6.6.7 and §38.34. The contributions change and the sum stays 4.00.
#[test]
fn real_sum_deposit_releases_when_driver_values_change() {
    check_deposit(
        SUM_DRIVERS,
        &["initial 4.00", "deposited -4.50", "drivers changed 4.00"],
    );
}

/// IEEE 1800-2023 §6.6.3 and §38.34. One contribution changes and the OR stays 27.
#[test]
fn wor_deposit_releases_when_a_driver_value_changes() {
    check_deposit(
        WOR_DRIVERS,
        &["initial 27", "deposited 4a", "driver changed 27"],
    );
}

/// IEEE 1800-2023 §6.6.1 and §38.34.
#[test]
fn direct_copy_wire_deposit_releases_when_source_changes() {
    check_deposit(
        DIRECT_COPY,
        &["initial 25", "deposited 4a", "source changed 17"],
    );
}

/// IEEE 1800-2023 §10.3 and §38.34.
#[test]
fn continuous_reader_sees_the_deposit() {
    check_deposit(
        CONTINUOUS_READER,
        &["initial 2.75 2.75", "deposited -4.50 -4.50"],
    );
}
