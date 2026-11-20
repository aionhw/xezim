//! IEEE 1800-2023 §21.3.4.3 (and reference display-side `%0` convention
//! from §21.2.1.3): `$fscanf` / `$sscanf` with an explicit `0` field width
//! (`%0d`, `%0f`, `%0s`, `%0h`, etc.) treats width 0 as unbounded rather than
//! matching zero characters, and `%*` suppresses assignment even with width 0.

use xezim::simulate;

#[test]
fn scanf_and_sscanf_zero_field_width_and_suppression() {
    let dir = std::env::temp_dir().join(format!("xezim_scan_zero_w_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scan_file = dir.join("scan.txt");
    let scan_file_sv = scan_file.to_string_lossy().replace('\\', "/");

    let src = format!(
        r#"module top;
  int fd, n, i0, iw;
  int iv_d0, iv_dw, iv_i0, iv_iw, iv_h0, iv_hw, iv_x0, iv_xw, iv_o0, iv_ow, iv_b0, iv_bw;
  byte c0, cw;
  real rf0, rfw, re0, rew, rg0, rgw, r2, rt5;
  realtime rt, rt_w;
  string s0, sw, s_pair;
  int trunc_d, skip_d, skip_s;
  initial begin
    // Every conversion with 0 width matches width-less form
    n = $sscanf("-42", "%0d", iv_d0);
    if (n != 1) $display("FAIL d0 n=%0d", n);
    void'($sscanf("-42", "%d", iv_dw));

    n = $sscanf("99", "%0i", iv_i0);
    if (n != 1) $display("FAIL i0 n=%0d", n);
    void'($sscanf("99", "%i", iv_iw));

    n = $sscanf("1a2b", "%0h", iv_h0);
    if (n != 1) $display("FAIL h0 n=%0d", n);
    void'($sscanf("1a2b", "%h", iv_hw));

    n = $sscanf("dead", "%0x", iv_x0);
    if (n != 1) $display("FAIL x0 n=%0d", n);
    void'($sscanf("dead", "%x", iv_xw));

    n = $sscanf("755", "%0o", iv_o0);
    if (n != 1) $display("FAIL o0 n=%0d", n);
    void'($sscanf("755", "%o", iv_ow));

    n = $sscanf("10110", "%0b", iv_b0);
    if (n != 1) $display("FAIL b0 n=%0d", n);
    void'($sscanf("10110", "%b", iv_bw));

    n = $sscanf("123.45", "%0f", rf0);
    if (n != 1) $display("FAIL f0 n=%0d", n);
    void'($sscanf("123.45", "%f", rfw));

    n = $sscanf("2.5e3", "%0e", re0);
    if (n != 1) $display("FAIL e0 n=%0d", n);
    void'($sscanf("2.5e3", "%e", rew));

    n = $sscanf("6.25", "%0g", rg0);
    if (n != 1) $display("FAIL g0 n=%0d", n);
    void'($sscanf("6.25", "%g", rgw));

    n = $sscanf("hello", "%0s", s0);
    if (n != 1) $display("FAIL s0 n=%0d", n);
    void'($sscanf("hello", "%s", sw));

    n = $sscanf("Z", "%0c", c0);
    if (n != 1) $display("FAIL c0 n=%0d", n);
    void'($sscanf("Z", "%c", cw));

    $display("EQ %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d %0d",
      iv_d0 == iv_dw && iv_d0 == -42,
      iv_i0 == iv_iw && iv_i0 == 99,
      iv_h0 == iv_hw && iv_h0 == 32'h1a2b,
      iv_x0 == iv_xw && iv_x0 == 32'hdead,
      iv_o0 == iv_ow && iv_o0 == 9'o755,
      iv_b0 == iv_bw && iv_b0 == 5'b10110,
      rf0 == rfw,
      re0 == rew,
      rg0 == rgw,
      s0 == sw && s0 == "hello",
      c0 == cw && c0 == "Z"
    );

    // Multi-field %0f %0s
    r2 = 0.0; s_pair = "";
    n = $sscanf("12.5 abc", "%0f %0s", r2, s_pair);
    $display("PAIR n=%0d r=%.1f s=%s", n, r2, s_pair);

    // Positive widths still truncate
    n = $sscanf("1794778382", "%3d", trunc_d);
    $display("TRUNC_D n=%0d v=%0d", n, trunc_d);
    n = $sscanf("1794778382", "%5f", rt5);
    $display("TRUNC_F n=%0d v=%0d", n, $rtoi(rt5));

    // Suppression with zero width (%*0d) and normal width (%*d)
    skip_d = 0;
    n = $sscanf("11 22", "%*0d %d", skip_d);
    $display("SKIP0 n=%0d v=%0d", n, skip_d);
    skip_s = 0;
    n = $sscanf("alpha 77", "%*0s %0d", skip_s);
    $display("SKIP_S n=%0d v=%0d", n, skip_s);

    // $fscanf into realtime from a file written in SV
    fd = $fopen("{scan_file_sv}", "w");
    $fdisplay(fd, "1794778382");
    $fclose(fd);

    fd = $fopen("{scan_file_sv}", "r");
    rt = 0.0;
    n = $fscanf(fd, "%0f", rt);
    $fclose(fd);
    fd = $fopen("{scan_file_sv}", "r");
    rt_w = 0.0;
    i0 = $fscanf(fd, "%f", rt_w);
    $fclose(fd);
    $display("FSCANF n=%0d eq=%0d", n, (i0 == 1) && (rt == rt_w) && (rt > 1794778381.0) && (rt < 1794778383.0));

    fd = $fopen("{scan_file_sv}", "r");
    iw = 0;
    n = $fscanf(fd, "%0d", iw);
    $fclose(fd);
    $display("FSCANF_D n=%0d iw=%0d", n, iw);
    $finish;
  end
endmodule
"#
    );

    let sim = simulate(&src, 1000).expect("simulate failed");
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect();
    let joined = lines.join("\n");

    for exp in [
        "EQ 1 1 1 1 1 1 1 1 1 1 1",
        "PAIR n=2 r=12.5 s=abc",
        "TRUNC_D n=1 v=179",
        "TRUNC_F n=1 v=17947",
        "SKIP0 n=1 v=22",
        "SKIP_S n=1 v=77",
        "FSCANF n=1 eq=1",
        "FSCANF_D n=1 iw=1794778382",
    ] {
        assert!(
            lines.iter().any(|l| l == exp),
            "missing line {exp:?}:\n{joined}"
        );
    }
}

#[test]
fn scanf_four_state_integer_digits() {
    let sim = simulate(r#"
module top;
  logic [3:0] binary_value;
  logic [7:0] hex_value, oct_value, dec_value;
  int n;
  initial begin
    n = $sscanf("Xx1? x1 1z Z", "%b %h %o %d", binary_value, hex_value, oct_value, dec_value);
    $display("%0d %b %b %b %b", n, binary_value, hex_value, oct_value, dec_value);
  end
endmodule
"#, 100).unwrap();
    assert!(sim.output.iter().any(|line| line.message == "4 xx1z xxxx0001 00001zzz zzzzzzzz"));
}
