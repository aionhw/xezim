//! IEEE 1800-2023 §21.3.4.3, Table 21-7: a floating-point field ends at
//! the next inappropriate character or the maximum field width.

use xezim::simulate;

const STRING_BOUNDARIES: &str = r#"
module top;
  real lo, hi;
  int n;
  string tail;
  initial begin
    n = $sscanf("0.9-1.1", "%f-%f", lo, hi);
    $display("minus=%0d", n == 2 && lo == 0.9 && hi == 1.1);
    n = $sscanf("1.5e-3+2", "%e+%g", lo, hi);
    $display("exponent=%0d", n == 2 && lo == 0.0015 && hi == 2.0);
    n = $sscanf("3.3-0.1", "%f%f", lo, hi);
    $display("adjacent=%0d", n == 2 && lo == 3.3 && hi == -0.1);
    n = $sscanf("1.5-", "%f", lo);
    $display("sign=%0d", n == 1 && lo == 1.5);
    n = $sscanf("2.5.", "%g", lo);
    $display("dot=%0d", n == 1 && lo == 2.5);
    n = $sscanf("1.2.3", "%f.%f", lo, hi);
    $display("dot_separator=%0d", n == 2 && lo == 1.2 && hi == 3.0);
    n = $sscanf("1.5-2", "%4f%s", lo, tail);
    $display("width=%0d", n == 2 && lo == 1.5 && tail == "-2");
    n = $sscanf("1.25-2.5", "%*f%g", hi);
    $display("suppression=%0d", n == 1 && hi == -2.5);
  end
endmodule
"#;

const SCAN_GUARDS: &str = r#"
module top;
  real lo, hi;
  int n, a, b;
  string tail;
  initial begin
    n = $sscanf("0.9:1.1", "%f:%f", lo, hi);
    $display("colon=%0d", n == 2 && lo == 0.9 && hi == 1.1);
    n = $sscanf("  +0.9 -1.1", "%f %f", lo, hi);
    $display("space=%0d", n == 2 && lo == 0.9 && hi == -1.1);
    n = $sscanf("  -12.5", "%3f%s", lo, tail);
    $display("width=%0d", n == 2 && lo == -12.0 && tail == ".5");
    n = $sscanf("1E2V", "%3f%s", lo, tail);
    $display("full_exp=%0d", n == 2 && lo == 100.0 && tail == "V");
    n = $sscanf("3.3V", "%fV", lo);
    $display("unit=%0d", n == 1 && lo == 3.3);
    n = $sscanf("0x1.8p1", "%f%s", lo, tail);
    $display("hex_prefix=%0d", n == 2 && lo == 0.0 && tail == "x1.8p1");
    n = $sscanf("5-3", "%d-%d", a, b);
    $display("integer=%0d", n == 2 && a == 5 && b == 3);
    lo = -9.0;
    tail = "unset";
    n = $sscanf("1e+2", "%3f%s", lo, tail);
    $display("width_in_exponent=%0d", n == 0 && lo == -9.0 && tail == "unset");
    n = $sscanf("2.5e-V", "%f%s", lo, tail);
    $display("incomplete_exp=%0d", n == 0 && lo == -9.0 && tail == "unset");
    n = $sscanf("+.e2", "%f", lo);
    $display("no_digits=%0d", n == 0 && lo == -9.0);
  end
endmodule
"#;

const FILE_BOUNDARIES: &str = r#"
module top;
  real lo, hi;
  int fd, n, delim;
  initial begin
    fd = $fopen("SCAN_FILE", "r");
    if (fd == 0) $fatal(1, "could not open scan input");
    lo = -9.0;
    n = $fscanf(fd, "%f", lo);
    delim = $fgetc(fd);
    $display("file_incomplete_exp=%0d", n == 0 && lo == -9.0 && delim == 8'h56);
    n = $fscanf(fd, "%f: %f", lo, hi);
    $display("file_across_lines=%0d", n == 2 && lo == 1.5 && hi == 2.5);
    n = $fscanf(fd, "%f-%f", lo, hi);
    $display("file_minus=%0d", n == 2 && lo == 0.9 && hi == 1.1);
    n = $fscanf(fd, "%g", lo);
    delim = $fgetc(fd);
    $display("unread=%0d", n == 1 && lo == 2.5 && delim == 8'h2e);
    while (delim != "\n" && delim != -1) delim = $fgetc(fd);
    // The literal '-' matches before the format's space consumes the newline.
    n = $fscanf(fd, "%f- %f", lo, hi);
    $display("file_sign_across_lines=%0d", n == 2 && lo == 1.5 && hi == 2.5);
    $fclose(fd);
  end
endmodule
"#;

fn check(src: &str, checks: &[&str]) {
    let sim = simulate(src, 100).expect("simulate failed");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    for name in checks {
        assert!(lines.contains(&format!("{name}=1").as_str()), "{lines:?}");
    }
}

#[test]
fn sscanf_real_field_boundaries() {
    check(
        STRING_BOUNDARIES,
        &[
            "minus",
            "exponent",
            "adjacent",
            "sign",
            "dot",
            "dot_separator",
            "width",
            "suppression",
        ],
    );
}

#[test]
fn sscanf_real_field_guards() {
    check(
        SCAN_GUARDS,
        &[
            "colon",
            "space",
            "width",
            "full_exp",
            "unit",
            "hex_prefix",
            "integer",
            "width_in_exponent",
            "incomplete_exp",
            "no_digits",
        ],
    );
}

#[test]
fn fscanf_real_field_boundaries() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("xezim_scan_real_{}_{stamp}", std::process::id()));
    std::fs::write(&path, "2.5e-V\n1.5:\n2.5\n0.9-1.1\n2.5.-\n1.5-\n2.5\n").unwrap();
    let path_sv = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let result = simulate(&FILE_BOUNDARIES.replace("SCAN_FILE", &path_sv), 100);
    std::fs::remove_file(path).unwrap();
    let sim = result.expect("simulate failed");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(
        lines,
        [
            "file_incomplete_exp=1",
            "file_across_lines=1",
            "file_minus=1",
            "unread=1",
            "file_sign_across_lines=1",
        ]
    );
}
