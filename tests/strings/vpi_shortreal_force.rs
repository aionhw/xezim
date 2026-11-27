use std::path::{Path, PathBuf};
use std::process::Command;

const SV: &str = r#"
`timescale 1ns/1ps
module top;
  import "DPI-C" context function void force_values();
  import "DPI-C" context function void reforce_values();
  import "DPI-C" context function void release_values();
  import "DPI-C" context function void check_shortreal_edges();

  nettype shortreal ShortNet;
  nettype real RealNet;
  shortreal sr, deposited, sv_forced, edge_value;
  struct { shortreal member; int tag; } payload;
  real rv, drive = 1.0;
  ShortNet sn;
  RealNet rn;
  logic [7:0] bits;
  assign sn = shortreal'(drive);
  assign rn = drive;

  initial begin
    #1;
    force sv_forced = 16777216.25;
    force_values();
    if (deposited != 16777216.0 || sv_forced != 16777216.0)
      $fatal(1, "shortreal deposit or SV force: %.17g %.17g", deposited, sv_forced);

    reforce_values();
    sr = 2.0;
    drive = 3.0;
    #1;
    if (sr != 16777216.0 || sn != 16777216.0 || payload.member != 16777216.0)
      $fatal(1, "shortreal re-force: %.17g %.17g %.17g", sr, sn, payload.member);
    if (rv != 16777216.25 || rn != 16777216.25 || bits !== 8'ha5)
      $fatal(1, "real or integral force: %.17g %.17g %h", rv, rn, bits);

    release_values();
    #1;
    if (sn != 3.0 || rn != 3.0)
      $fatal(1, "released nets did not return to their drivers: %.17g %.17g", sn, rn);
    if (sr != 16777216.0 || payload.member != 16777216.0)
      $fatal(1, "released variables lost their last forced value: %.17g %.17g", sr, payload.member);
    sr = 4.0;
    payload.member = 5.0;
    if (sr != 4.0 || payload.member != 5.0)
      $fatal(1, "released variables did not accept assignments: %.17g %.17g", sr, payload.member);

    check_shortreal_edges();
    $display("SHORTREAL_FORCE_PASS");
    $finish;
  end
endmodule
"#;

const C: &str = r#"
#include <math.h>
#include "vpi_user.h"

static const char *const shortreal_targets[] = {"top.sr", "top.sn", "top.payload.member"};

static vpiHandle find_signal(const char *name) {
    vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)name, NULL);
    if (!h) vpi_printf("VPI FAIL: missing %s\n", name);
    return h;
}

static void put_and_check(const char *name, s_vpi_value input, int flag, double expected) {
    vpiHandle h = find_signal(name);
    if (!h) return;
    vpi_put_value(h, &input, NULL, flag);

    s_vpi_value result = { .format = vpiRealVal };
    vpi_get_value(h, &result);
    if (result.format != vpiRealVal) {
        vpi_printf("VPI FAIL: %s read returned format %d\n", name, result.format);
    } else {
        double actual = result.value.real;
        int matches = isnan(expected) ? isnan(actual)
            : actual == expected && !!signbit(actual) == !!signbit(expected);
        if (!matches) {
            vpi_printf("VPI FAIL: %s write flag %d format %d read %.17g, expected %.17g\n",
                       name, flag, input.format, actual, expected);
        }
    }
    vpi_free_object(h);
}

void force_values(void) {
    s_vpi_value input = { .format = vpiRealVal, .value.real = 16777216.25 };
    for (size_t i = 0; i < sizeof shortreal_targets / sizeof shortreal_targets[0]; ++i) {
        put_and_check(shortreal_targets[i], input, vpiForceFlag, 16777216.0);
    }
    put_and_check("top.rv", input, vpiForceFlag, 16777216.25);
    put_and_check("top.rn", input, vpiForceFlag, 16777216.25);
    put_and_check("top.deposited", input, vpiNoDelay, 16777216.0);

    vpiHandle h = find_signal("top.bits");
    if (h) {
        s_vpi_value bits = { .format = vpiIntVal, .value.integer = 0xa5 };
        vpi_put_value(h, &bits, NULL, vpiForceFlag);
        vpi_free_object(h);
    }
}

void reforce_values(void) {
    s_vpi_value negative = { .format = vpiRealVal, .value.real = -16777216.25 };
    s_vpi_value integer = { .format = vpiIntVal, .value.integer = 16777217 };
    for (size_t i = 0; i < sizeof shortreal_targets / sizeof shortreal_targets[0]; ++i) {
        put_and_check(shortreal_targets[i], negative, vpiForceFlag, -16777216.0);
        put_and_check(shortreal_targets[i], integer, vpiForceFlag, 16777216.0);
    }
}

static void release_signal(const char *name) {
    vpiHandle h = find_signal(name);
    if (h) {
        vpi_put_value(h, NULL, NULL, vpiReleaseFlag);
        vpi_free_object(h);
    }
}

void release_values(void) {
    for (size_t i = 0; i < sizeof shortreal_targets / sizeof shortreal_targets[0]; ++i) {
        release_signal(shortreal_targets[i]);
    }
    release_signal("top.rn");
}

void check_shortreal_edges(void) {
    const struct { double input, expected; } cases[] = {
        { -0.0, -0.0 },
        { NAN, NAN },
        { INFINITY, INFINITY },
        { -INFINITY, -INFINITY },
        { 0x1.8p-150, 0x1p-149 }, // Rounds to the smallest binary32 subnormal.
        { -0x1p-151, -0.0 },
        { 0x1p128, INFINITY },
        { -0x1p128, -INFINITY },
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; ++i) {
        s_vpi_value input = { .format = vpiRealVal, .value.real = cases[i].input };
        put_and_check("top.edge_value", input, vpiForceFlag, cases[i].expected);
    }
}
"#;

/// IEEE 1800-2023 §6.12, §10.6.2 and §38.34.
#[test]
fn vpi_force_rounds_shortreal_values() {
    let scratch = Command::new("mktemp")
        .arg("-d")
        .arg(std::env::temp_dir().join("xezim-vpi-shortreal-force.XXXXXX"))
        .output()
        .expect("failed to launch mktemp");
    assert!(scratch.status.success(), "{scratch:?}");
    let dir = PathBuf::from(String::from_utf8(scratch.stdout).unwrap().trim_end());
    std::fs::write(dir.join("force.c"), C).unwrap();
    std::fs::write(dir.join("top.sv"), SV).unwrap();

    let mut cc = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()));
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    let built = cc
        .arg("-I")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("include"))
        .args(["force.c", "-o", "force.so"])
        .current_dir(&dir)
        .output()
        .expect("failed to launch C compiler");
    assert!(built.status.success(), "{built:?}");

    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "--dpi-lib", "./force.so", "-s", "top", "top.sv"])
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
    assert!(!text.contains("VPI FAIL:"), "{text}");
    assert!(
        text.lines().any(|line| line == "SHORTREAL_FORCE_PASS"),
        "{text}"
    );
}
