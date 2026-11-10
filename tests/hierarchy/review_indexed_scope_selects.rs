//! §9.4.2, §10.3.2, §6.10: a path that enters a scope through an index and
//! ends in a bit or part select (`g[1].u.c[0]`, `g[1].u.c[2:1]`), as an event
//! control and as a continuous-assign target, and the generate block named
//! at the root of such a path.
//!
//! The indexed-scope fold named only a whole leaf, so a select on the leaf
//! kept the unfolded path: the event control armed on nothing and fired once
//! at time 0, and the partial assignments drove nothing. Elaboration also
//! took the generate block `g` of `assign g[0].u.a = s;` for an undeclared
//! identifier and declared a 1-bit implicit net `g`, with a warning.

use xezim::simulate;

const SELECTS: &str = r#"
module leaf (input logic clk, output logic [3:0] c);
  logic [3:0] n = 0;
  always @(posedge clk) n <= n + 1;
  assign c = n;
endmodule
module sink (input logic [3:0] a, output logic [3:0] c);
  assign c = a;
endmodule
module tb;
  logic clk = 0;
  always #5 clk = ~clk;
  for (genvar i = 0; i < 2; i++) begin : g
    leaf u (.clk(clk), .c());
  end
  leaf v (.clk(clk), .c());
  integer n_bit_v = 0, n_bit_g = 0, n_bit_rooted = 0, n_part_g = 0;
  // `v.c[0]` is the control that worked before.
  always @(posedge v.c[0]) n_bit_v++;
  always @(posedge g[1].u.c[0]) n_bit_g++;
  always @(posedge tb.g[1].u.c[1]) n_bit_rooted++;
  always @(g[1].u.c[2:1]) n_part_g++;
  // Each input port is left unconnected: these assigns are its only drivers.
  for (genvar i = 0; i < 2; i++) begin : ps
    sink u (.c());
  end
  assign tb.ps[0].u.a[0] = 1'b1;
  assign tb.ps[0].u.a[3:1] = 3'b011;
  assign ps[1].u.a[2:0] = 3'b101;
  assign ps[1].u.a[3] = 1'b1;
  initial begin
    #57;
    $display("R bit v=%0d g=%0d rooted=%0d part=%0d", n_bit_v, n_bit_g, n_bit_rooted, n_part_g);
    $display("R ps %h %h", ps[0].u.c, ps[1].u.c);
    $finish;
  end
endmodule
"#;

/// Counts and values as the reference simulator gives them: `c` steps 1..6
/// on the clock edges up to time 57.
#[test]
fn indexed_scope_leaf_selects_fire_and_drive() {
    let out: Vec<String> = simulate(SELECTS, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(
        out,
        vec!["R bit v=3 g=3 rooted=2 part=4", "R ps 7 d"],
        "{out:?}"
    );
}

#[test]
fn indexed_scope_root_declares_no_implicit_net() {
    let dir = std::env::temp_dir().join(format!("xezim_indexed_scope_root_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("root.sv");
    std::fs::write(
        &sv,
        r#"
module leaf (input logic [3:0] a, output logic [3:0] c);
  assign c = a;
endmodule
module tb;
  logic [3:0] s = 4'h3, y;
  for (genvar i = 0; i < 2; i++) begin : g
    leaf u (.c());
  end
  assign g[0].u.a = s;
  assign y = g[0].u.c;
  initial begin #1 $display("R y=%h", y); $finish; end
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
        stdout.contains("R y=3"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("implicit 1-bit net") && !stdout.contains("implicit 1-bit net"),
        "spurious implicit-net warning:\n{stderr}{stdout}"
    );
}
