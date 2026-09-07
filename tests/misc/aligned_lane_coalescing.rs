//! Aligned bit-sliced gates may execute as a single vector operation. Keep the
//! optimization opt-in while checking that its four-state results match the
//! scalar path exactly.

use std::process::Command;

#[test]
fn aligned_lanes_match_scalar_four_state_results() {
    let src = r#"
module lane_check;
  logic [31:0] left, right;
  logic [63:0] wide_left, wide_right;
  wire [31:0] copied, flipped, both, neither, either, nor_value;
  wire [31:0] different, same;
  wire [31:0] selected, scattered;
  wire [63:0] wide_mix;
  wire [47:0] shifted;

  genvar bit_no;
  generate
    for (bit_no = 0; bit_no < 32; bit_no++) begin : lanes
      assign copied[bit_no]    = left[bit_no];
      assign flipped[bit_no]   = ~left[bit_no];
      assign both[bit_no]      = left[bit_no] & right[bit_no];
      assign neither[bit_no]   = ~(left[bit_no] & right[bit_no]);
      assign either[bit_no]    = left[bit_no] | right[bit_no];
      assign nor_value[bit_no] = ~(left[bit_no] | right[bit_no]);
      assign different[bit_no] = left[bit_no] ^ right[bit_no];
      assign same[bit_no]      = ~(left[bit_no] ^ right[bit_no]);
      assign shifted[bit_no+8] = left[bit_no] | right[bit_no];
      assign selected[bit_no] = left[bit_no] ? right[bit_no] : copied[bit_no];
      wire lane_hold;
      assign lane_hold = ~right[bit_no];
      assign scattered[bit_no] = lane_hold;
    end
    for (bit_no = 0; bit_no < 64; bit_no++) begin : wide_lanes
      assign wide_mix[bit_no] = wide_left[bit_no] ^ wide_right[bit_no];
    end
  endgenerate

  task automatic check_outputs;
    #1;
    if (copied !== left || flipped !== ~left || both !== (left & right) ||
        neither !== ~(left & right) || either !== (left | right) ||
        nor_value !== ~(left | right) || different !== (left ^ right) ||
        same !== ~(left ^ right) || wide_mix !== (wide_left ^ wide_right) ||
        shifted[39:8] !== (left | right) || scattered !== ~right) begin
      $fatal(1, "lane result mismatch");
    end
    for (int lane_no = 0; lane_no < 32; lane_no++) begin
      if (selected[lane_no] !== (left[lane_no] ? right[lane_no] : copied[lane_no]))
        $fatal(1, "selected lane mismatch");
    end
    $display("RESULT %h %h %h %h %h %h %h %h %h %h %h %h",
             copied, flipped, both, neither, either, nor_value, different,
             same, wide_mix, shifted[39:8], selected, scattered);
  endtask

  initial begin
    $dumpfile("lane_trace.vcd");
    $dumpvars(0, lane_check);
    left = 32'h00000000; right = 32'hffffffff;
    wide_left = 64'h0123456789abcdef; wide_right = 64'hfedcba9876543210;
    check_outputs();
    left = 32'ha5a55a5a; right = 32'h3cc3f00f;
    wide_left = 64'ha55aa55a5aa55aa5; wide_right = 64'h0ff00ff0f00ff00f;
    check_outputs();
    left = 32'hxxxxxxxx; right = 32'h0000ffff;
    wide_left = 64'hxxxx0000zzzz1111; wide_right = 64'h0000ffff1111zzzz;
    check_outputs();
    left = 32'hzzzzzzzz; right = 32'hffff0000;
    wide_left = 64'hzzzzzzzzzzzzzzzz; wide_right = 64'hffff00000000ffff;
    check_outputs();
    left = 32'h10xz01zx; right = 32'hz10x1z0x;
    wide_left = 64'h10xz01zx10xz01zx; wide_right = 64'hz10x1z0xz10x1z0x;
    check_outputs();
    $display("LANES_OK");
    $finish;
  end
endmodule
"#;

    let dir = std::env::temp_dir().join(format!("xezim_aligned_lanes_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("lane_check.sv");
    std::fs::write(&source, src).unwrap();

    let run = |coalesce: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.args(["--no-cache", "-s", "lane_check", "--max-time", "1000"])
            .arg(&source)
            .current_dir(&dir)
            .env_remove("XEZIM_VEC_COALESCE")
            .env_remove("XEZIM_VEC_SCATTER")
            .env_remove("XEZIM_VEC_SCATTER_MIN")
            .env_remove("XEZIM_COMMIT_PLAN")
            .env_remove("XEZIM_VEC_STATS");
        if coalesce {
            cmd.env("XEZIM_VEC_COALESCE", "1")
                .env("XEZIM_VEC_SCATTER", "1")
                .env("XEZIM_VEC_SCATTER_MIN", "32")
                .env("XEZIM_COMMIT_PLAN", "1")
                .env("XEZIM_VEC_STATS", "1");
        }
        cmd.output().expect("run lane check")
    };

    let scalar = run(false);
    let vector = run(true);
    assert!(scalar.status.success(), "scalar run failed: {:?}", scalar);
    assert!(vector.status.success(), "vector run failed: {:?}", vector);

    let result_lines = |out: &[u8]| {
        String::from_utf8_lossy(out)
            .lines()
            .filter(|line| line.starts_with("RESULT ") || *line == "LANES_OK")
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let scalar_results = result_lines(&scalar.stdout);
    assert_eq!(scalar_results.len(), 6, "missing scalar results");
    assert_eq!(result_lines(&vector.stdout), scalar_results);

    let vector_log = String::from_utf8_lossy(&vector.stderr);
    assert!(
        vector_log.contains("[VEC] vectors=11 bits=384 entries_removed=373"),
        "unexpected coalescing summary: {vector_log}"
    );

    let formats = [
        (vec!["--wave"], dir.join("lane_trace.vcd")),
        (vec!["--fst", "lane_trace.fst"], dir.join("lane_trace.fst")),
        (
            vec!["--xtrace", "lane_trace.xt"],
            dir.join("lane_trace.xt"),
        ),
    ];
    for (args, artifact) in formats {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(args)
            .args(["--no-cache", "-s", "lane_check", "--max-time", "1000"])
            .arg(&source)
            .current_dir(&dir)
            .env("XEZIM_VEC_COALESCE", "1")
            .env("XEZIM_VEC_SCATTER", "1")
            .env("XEZIM_VEC_SCATTER_MIN", "32")
            .env("XEZIM_COMMIT_PLAN", "1")
            .output()
            .expect("run lane trace check");
        assert!(out.status.success(), "trace run failed: {out:?}");
        assert_eq!(result_lines(&out.stdout), scalar_results);
        let size = std::fs::metadata(&artifact)
            .unwrap_or_else(|_| panic!("missing trace artifact: {}", artifact.display()))
            .len();
        assert!(size > 64, "trace artifact is too small: {}", artifact.display());
    }
}
