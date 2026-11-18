use std::path::{Path, PathBuf};
use std::process::Command;

const DEPOSIT_C: &str = r#"
#include "vpi_user.h"

static PLI_INT32 changed(p_cb_data cb) {
    vpi_printf("T|callback %s %.2f\n", vpi_get_str(vpiName, cb->obj), cb->value->value.real);
    return 0;
}

void deposit(const char *path) {
    vpiHandle n = vpi_handle_by_name((PLI_BYTE8 *)path, NULL);
    s_vpi_value value = { .format = vpiRealVal };
    s_vpi_time time = { .type = vpiSuppressTime };
    s_cb_data cb = { .reason = cbValueChange, .cb_rtn = changed, .obj = n,
                     .time = &time, .value = &value };
    vpi_register_cb(&cb);

    value.value.real = -4.5;
    vpi_put_value(n, &value, NULL, vpiNoDelay);
    vpi_get_value(n, &value);
    vpi_printf("T|deposit %s %.2f\n", vpi_get_str(vpiName, n), value.value.real);
}
"#;

const PACKAGE_RESOLVER: &str = r#"
`timescale 1ns/1ps
package nets;
  function automatic real f(input real drivers[]);
    f = 0.0;
    foreach (drivers[i]) f += drivers[i];
  endfunction
  nettype real N with f;
endpackage
module top;
  import "DPI-C" context function void deposit(input string path);
  import nets::N;
  real d;
  N n;
  assign n = d;
  initial begin
    d = 2.75;
    #1 $display("T|initial %.2f", n);
    #1 deposit("top.n");
    #1 $display("T|held %.2f", n);
    #5 d = -1.25;
    #1 $display("T|changed %.2f", n);
    $finish;
  end
endmodule
"#;

const SAME_NAME_RESOLVER: &str = r#"
`timescale 1ns/1ps
package nets;
  function automatic real f(input real drivers[]);
    f = 0.0;
    foreach (drivers[i]) f += drivers[i];
  endfunction
  nettype real N with f;
endpackage
module top;
  import "DPI-C" context function void deposit(input string path);
  import nets::N;
  real d, f;
  N n;
  assign n = d;
  initial begin
    d = 2.75;
    #1 $display("T|initial %.2f", n);
    #1 deposit("top.n");
    #1 $display("T|held %.2f", n);
    #1 f = 12.5;
    #1 $display("T|after unrelated %.2f", n);
    #3 d = -1.25;
    #1 $display("T|changed %.2f", n);
    $finish;
  end
endmodule
"#;

const RESOLVER_GUARDS: &str = r#"
`timescale 1ns/1ps
package nets;
  function automatic real accumulated(input real drivers[]);
    real acc;
    acc = 0.0;
    foreach (drivers[i]) acc += drivers[i];
    return acc;
  endfunction
  nettype real Pkg with accumulated;
endpackage
module top;
  import "DPI-C" context function void deposit(input string path);
  function automatic real local_resolve(input real drivers[]);
    local_resolve = 0.0;
    foreach (drivers[i]) local_resolve += drivers[i];
  endfunction
  nettype real Local with local_resolve;
  import nets::Pkg;
  real d;
  Local local_n;
  Pkg n;
  assign n = d;
  assign local_n = d;
  initial begin
    d = 2.75;
    #1 $display("T|initial %.2f %.2f", n, local_n);
    #1 begin
      deposit("top.n");
      deposit("top.local_n");
    end
    #1 $display("T|held %.2f %.2f", n, local_n);
    #5 d = -1.25;
    #1 $display("T|changed %.2f %.2f", n, local_n);
    $finish;
  end
endmodule
"#;

const SAME_NAME_PACKAGES: &str = r#"
`timescale 1ns/1ps
package sums;
  function automatic real f(input real drivers[]);
    f = 0.0;
    foreach (drivers[i]) f += drivers[i];
  endfunction
  nettype real Sum with f;
endpackage
package maxima;
  function automatic real f(input real drivers[]);
    f = 0.0;
    foreach (drivers[i])
      if (i == 0 || drivers[i] > f) f = drivers[i];
  endfunction
  nettype real Max with f;
endpackage
module top;
  import "DPI-C" context function void deposit(input string path);
  import sums::Sum;
  import maxima::Max;
  real d, e, f;
  Sum sum_n;
  Max max_n;
  assign sum_n = d;
  assign sum_n = e;
  assign max_n = d;
  assign max_n = e;
  initial begin
    d = 1.0;
    e = 2.5;
    #1 $display("T|initial %.2f %.2f", sum_n, max_n);
    #1 begin
      deposit("top.sum_n");
      deposit("top.max_n");
    end
    #1 $display("T|held %.2f %.2f", sum_n, max_n);
    #5 d = 3.0;
    #1 $display("T|changed %.2f %.2f", sum_n, max_n);
    $finish;
  end
endmodule
"#;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    dir
}

fn vpi_run(tag: &str, sv: &str) -> String {
    let dir = scratch(tag);
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
    text
}

fn check_lines(out: &str, prefix: &str, expected: &[&str]) {
    let got: Vec<_> = out
        .lines()
        .filter_map(|line| line.strip_prefix(prefix))
        .collect();
    assert_eq!(got, expected, "{out}");
}

/// IEEE 1800-2023 §6.6.7, §13.4.1, §38.34 and §38.36.1.
#[test]
fn package_resolver_deposit_holds_until_driver_changes() {
    let out = vpi_run("package_resolver", PACKAGE_RESOLVER);
    check_lines(&out, "T|callback n ", &["-4.50", "-1.25"]);
    check_lines(&out, "T|deposit n ", &["-4.50"]);
    check_lines(&out, "T|held ", &["-4.50"]);
    check_lines(&out, "T|changed ", &["-1.25"]);
}

/// IEEE 1800-2023 §13.4.1, §26.3 and §38.34. The module's `f` is unrelated
/// to the resolver's implicit result variable.
#[test]
fn deposit_survives_change_to_signal_named_like_package_resolver() {
    let out = vpi_run("same_name_resolver", SAME_NAME_RESOLVER);
    check_lines(&out, "T|callback n ", &["-4.50", "-1.25"]);
    check_lines(&out, "T|after unrelated ", &["-4.50"]);
}

/// IEEE 1800-2023 §6.6.7, §13.4.1 and §38.34. A module-local resolver and
/// a package resolver with an explicit local accumulator also hold deposits.
#[test]
fn module_resolver_and_explicit_accumulator_hold_deposits() {
    let out = vpi_run("resolver_guards", RESOLVER_GUARDS);
    check_lines(&out, "T|callback n ", &["-4.50", "-1.25"]);
    check_lines(&out, "T|callback local_n ", &["-4.50", "-1.25"]);
    check_lines(&out, "T|held ", &["-4.50 -4.50"]);
    check_lines(&out, "T|changed ", &["-1.25 -1.25"]);
}

/// IEEE 1800-2023 §6.6.7, §13.4.1, §26.3 and §38.34. Each nettype uses
/// its own package's resolver when both packages declare `f`. The module
/// signal `f` stays constant during this test.
#[test]
fn same_named_resolvers_in_different_packages_hold_deposits() {
    let out = vpi_run("same_name_packages", SAME_NAME_PACKAGES);
    check_lines(&out, "T|initial ", &["3.50 2.50"]);
    check_lines(&out, "T|callback sum_n ", &["-4.50", "5.50"]);
    check_lines(&out, "T|callback max_n ", &["-4.50", "3.00"]);
    check_lines(&out, "T|held ", &["-4.50 -4.50"]);
    check_lines(&out, "T|changed ", &["5.50 3.00"]);
}
