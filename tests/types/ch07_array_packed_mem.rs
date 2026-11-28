//! Part 2 self-tests for PackedMem (>=256 cells, 1-D/2-D/3-D, >64-bit wide,
//! 2-state masking, NBA, $readmemh) and BulkArrayCopy (>4096 cells,
//! packed/unpacked continuous assign chains, ascending/descending correspondence).

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulation should succeed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_has(out: &[String], expected: &str) {
    assert!(
        out.iter().any(|l| l == expected),
        "missing {expected:?} in {out:?}"
    );
}

#[test]
fn packed_mem_1d_nd_wide_twostate_nba_and_readmemh() {
    let hex_path = std::env::temp_dir().join(format!("xezim_pmem_{}.hex", std::process::id()));
    let mut hex_body = String::new();
    for i in 0..260u32 {
        hex_body.push_str(&format!("{:04x}\n", i + 0x1000));
    }
    std::fs::write(&hex_path, &hex_body).expect("write temp hex file");
    let hex_str = hex_path.to_string_lossy().replace('\\', "/");

    let src = format!(
        r#"
module top;
  bit   [15:0]  mem1 [512];        // >=256 cells, 2-state
  logic [31:0]  grid [16][32];     // 512 cells, 2-D, 4-state
  bit   [7:0]   cube [4][8][8];    // 256 cells, 3-D, 2-state
  logic [127:0] wide [300];        // 300 cells, >64-bit wide (2 words)
  bit   [15:0]  loaded [260];

  initial begin
    $readmemh("{hex_str}", loaded);

    // 2-state X/Z -> 0 masking in PackedMem store + NBA
    mem1[10] = 16'hx5a3;
    mem1[11] <= 16'hz3c1;
    // Out-of-bounds read on 2-state vs 4-state PackedMem
    grid[3][17] = 32'hDEAD_BEEF;
    cube[2][5][7] = 8'hxF;
    wide[250] = {{32'hCAFE_BABE, 32'h1234_5678, 32'h9ABC_DEF0, 32'h0102_0304}};

    #1;
    $display("PM1|%04x %04x oob=%04x", mem1[10], mem1[11], mem1[600]);
    $display("PM2|%08x oob=%08x", grid[3][17], grid[20][0]);
    $display("PM3|%02x oob=%02x", cube[2][5][7], cube[9][0][0]);
    $display("PMW|%08x_%08x", wide[250][127:96], wide[250][31:0]);
    $display("PMHEX|%04x %04x", loaded[0], loaded[259]);
  end
endmodule
"#
    );

    let out = sim_lines(&src);
    let _ = std::fs::remove_file(&hex_path);

    assert_has(&out, "PM1|05a3 03c1 oob=0000");
    assert_has(&out, "PM2|deadbeef oob=xxxxxxxx");
    assert_has(&out, "PM3|0f oob=00");
    assert_has(&out, "PMW|cafebabe_01020304");
    assert_has(&out, "PMHEX|1000 1103");
}

#[test]
fn bulk_array_copy_large_and_chained_continuous_assigns() {
    let out = sim_lines(
        r#"
module top;
  logic [15:0] big_a [0:4999];
  logic [15:0] big_b [0:4999];
  logic [15:0] big_c [0:4999];
  logic [15:0] small_u [0:3];

  // Whole >4096-cell chain: packed -> packed -> packed -> small unpacked
  assign big_b = big_a;
  assign big_c = big_b;
  assign small_u = big_c[4996:4999];

  // Descending / slice / 2-D row continuous copy on PackedMem
  logic [31:0] src_asc  [0:299];
  logic [31:0] dst_desc [299:0];
  assign dst_desc = src_asc;

  logic [31:0] sl_dst [0:3];
  assign sl_dst = src_asc[10:13];

  logic [31:0] mat [16][32];
  logic [31:0] row [32];
  assign row = mat[5];

  initial begin
    big_a[0] = 16'h1111;
    big_a[2500] = 16'h2222;
    big_a[4999] = 16'hx333;

    src_asc[0] = 32'hA000_0001;
    src_asc[10] = 32'h1010_0000;
    src_asc[13] = 32'h1313_0000;
    src_asc[299] = 32'hB000_0299;

    mat[5][0] = 32'h5500_0000;
    mat[5][31] = 32'h5500_0031;

    #1;
    $display("BIG|%04x %04x %04x u=%04x", big_c[0], big_c[2500], big_c[4999], small_u[3]);
    $display("DESC|%08x %08x", dst_desc[299], dst_desc[0]);
    $display("SL|%08x %08x", sl_dst[0], sl_dst[3]);
    $display("ROW|%08x %08x", row[0], row[31]);
  end
endmodule
"#,
    );
    assert_has(&out, "BIG|1111 2222 x333 u=x333");
    assert_has(&out, "DESC|a0000001 b0000299");
    assert_has(&out, "SL|10100000 13130000");
    assert_has(&out, "ROW|55000000 55000031");
}
