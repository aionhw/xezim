//! Issue #147: under `XEZIM_PACKED_MEM=1`, NBAs into packed-arena cells
//! committed IMMEDIATELY (`packed.set_raw`) — blocking semantics — so a
//! same-timestep reader saw the new value a delta early. They now queue in
//! `packed_nba` and mature in the NBA region. Building the test also
//! exposed two latent panics on arena ids (the blocking fast-write path and
//! the AST-eval array-read fast path indexed per-signal tables), both fixed.
//!
//! Runs the binary in a subprocess: the packed flag and the name threshold
//! are read per-Simulator from the environment, and in-process env mutation
//! races parallel tests.
use std::process::Command;

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

const RACE: &str = r#"module top;
  logic clk = 0; always #5 clk = ~clk;
  logic [7:0] mem [0:63];
  logic [5:0] wa = 0, ra = 0; logic [7:0] wd = 8'hAA; logic we = 0;
  logic [7:0] q;
  always @(posedge clk) begin
    if (we) mem[wa] <= wd;
    q <= mem[ra];
  end
  initial begin
    mem[5] = 8'h11;                       // blocking fast-path write (panicked)
    wa = 5; ra = 5; we = 1;
    @(posedge clk); #1;
    $display("R1_%h_%h", q, mem[5]);      // AST-path read (panicked)
    we = 0; @(posedge clk); #1;
    $display("R2_%h", q);
    $finish;
  end
endmodule
"#;

const CELL_TRAITS: &str = r#"module top;
  logic [7:0] plane_a [0:63];
  bit [7:0] plane_b [0:63];
  byte plane_c [0:63];
  int failures = 0;
  int slot;

  initial begin
    if (^plane_a[0] !== 1'bx) failures++;
    if (plane_b[0] !== 8'h00) failures++;
    plane_b[1] = 8'hxx;
    if (plane_b[1] !== 8'h00) failures++;
    plane_c[2] = -1;
    if (!(plane_c[2] < 0)) failures++;
    for (slot = 0; slot < 64; slot++) begin
      plane_a[slot] = slot[7:0] ^ 8'h5a;
    end
    if (plane_a[37] !== (8'd37 ^ 8'h5a)) failures++;
    if (^plane_a[64] !== 1'bx) failures++;
    $display("CELL_TRAITS failures=%0d value=%0h", failures, plane_a[37]);
    $finish;
  end
endmodule
"#;

#[test]
fn packed_nba_matures_in_nba_region() {
    let path = "/tmp/packed_nba_region_test.sv";
    std::fs::write(path, RACE).unwrap();
    let out = Command::new(xezim())
        .args(["--simulate", "-s", "top", path])
        .env("XEZIM_PACKED_MEM", "1")
        .env("XEZIM_LARGE_ARRAY_NAME_THRESHOLD", "16")
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Same-cycle read must see the OLD value (reference-verified R1_11_aa);
    // the immediate-commit bug read back AA in the same edge.
    assert!(stdout.contains("R1_11_aa"), "packed NBA leaked early:\n{stdout}");
    assert!(stdout.contains("R2_aa"), "{stdout}");
    // And identical behavior with packing OFF (control).
    let out2 = Command::new(xezim())
        .args(["--simulate", "-s", "top", path])
        .output()
        .expect("run xezim");
    let s2 = String::from_utf8_lossy(&out2.stdout);
    assert!(s2.contains("R1_11_aa") && s2.contains("R2_aa"), "{s2}");
}

#[test]
fn packed_cells_preserve_traits_and_compiled_writes() {
    let path = "/tmp/packed_cell_traits_test.sv";
    std::fs::write(path, CELL_TRAITS).unwrap();
    let run = |enabled: bool| {
        let mut cmd = Command::new(xezim());
        cmd.args(["--simulate", "-s", "top", path])
            .env("XEZIM_LARGE_ARRAY_NAME_THRESHOLD", "16")
            .env("XEZIM_PACKED_MEM", if enabled { "1" } else { "0" });
        cmd.output().expect("run xezim")
    };

    let packed = run(true);
    let regular = run(false);
    assert!(packed.status.success(), "packed run failed: {packed:?}");
    assert!(regular.status.success(), "regular run failed: {regular:?}");
    let packed_stdout = String::from_utf8_lossy(&packed.stdout);
    let regular_stdout = String::from_utf8_lossy(&regular.stdout);
    assert!(
        packed_stdout.contains("CELL_TRAITS failures=0 value=7f"),
        "{packed_stdout}"
    );
    assert!(
        regular_stdout.contains("CELL_TRAITS failures=0 value=7f"),
        "{regular_stdout}"
    );
}
