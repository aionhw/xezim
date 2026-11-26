//! A pure helper function called from a `for (int i = ...)` loop body must
//! not push the whole loop onto the AST interpreter.
//!
//! `compile_pure_call` has always been able to inline a function whose body
//! is a single assignment over input formals — the ubiquitous combinational
//! helper. Three independent restrictions still kept it from ever firing in
//! a loop inside an instantiated module, each hiding the next:
//!
//!   1. the loop gate rejected EVERY `Call`, without asking whether the
//!      compiler could inline it — a guard stricter than what it guarded;
//!   2. purity judged the function's OWN formals and result as foreign
//!      references, because elaboration rewrites an instantiated module's
//!      body to instance-qualified names (`u0.c`, `u0.onehot`) — so helpers
//!      were inlinable only in top-level modules;
//!   3. the block-local lookup refused any dotted name, so the qualified
//!      bindings the inliner installs could never be found.
//!
//! `vec[i] <= onehot(code[i])` is the shape this costs, and it is everywhere
//! in RTL. On the design that exposed it (34 instances of a 16-lane
//! pipeline) the loop fell back 336,566 times and the run took 3.2x longer.
//!
//! Values here are reference-verified, including through X.

use std::process::Command;

fn run(src: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_inlcall_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tb.sv");
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    text
}

#[test]
fn pure_call_in_loop_body_compiles_inside_an_instance() {
    // Two instances, so the function is registered instance-qualified —
    // the case that never inlined before.
    let text = run(r#"module leaf (input clk, input logic [15:0][3:0] code,
                 output logic [15:0][15:0] vec);
  function automatic [15:0] onehot (input [3:0] c);
    onehot = (1'b1 << c);
  endfunction
  always @(posedge clk) begin
    for (int i = 0; i < 16; i++) vec[i] <= onehot(code[i]);
  end
endmodule
module top;
  logic clk = 0; always #5 clk = ~clk;
  logic [15:0][3:0] code;
  wire  [15:0][15:0] v0, v1;
  leaf u0 (.clk(clk), .code(code), .vec(v0));
  leaf u1 (.clk(clk), .code(code), .vec(v1));
  integer k;
  initial begin
    for (k = 0; k < 16; k = k + 1) code[k] = k[3:0];
    code[7] = 4'bxx01;              // X must propagate through the inline
  end
  initial begin
    #200;
    $display("R v0_3=%0h v1_5=%0h vx=%0h", v0[3], v1[5], v0[7]);
    $finish;
  end
endmodule
"#);
    // 1<<3 and 1<<5; an X shift amount yields an all-X result. Reference-verified.
    assert!(
        text.contains("R v0_3=8 v1_5=20 vx=xxxx"),
        "inlined helper produced wrong values:\n{text}"
    );
    // And the loop must be COMPILED — that is the whole point.
    for reason in ["For_init_vardecl", "blocking_target", "Expr_Call"] {
        assert!(
            !text.contains(reason),
            "loop fell back with `{reason}`:\n{text}"
        );
    }
}

#[test]
fn impure_helper_still_refused() {
    // Reads a module signal, so it is NOT pure in its arguments and must not
    // be inlined; the run must still be correct via the interpreter.
    let text = run(r#"module top;
  logic clk = 0; always #5 clk = ~clk;
  logic [3:0] bias = 4'd2;
  logic [15:0][3:0] src, dst;
  function automatic [3:0] add_bias (input [3:0] c);
    add_bias = c + bias;          // free reference to a module signal
  endfunction
  integer k;
  initial for (k = 0; k < 16; k = k + 1) src[k] = k[3:0];
  always @(posedge clk) begin
    for (int i = 0; i < 16; i++) dst[i] <= add_bias(src[i]);
  end
  initial begin #200; $display("I dst3=%0h dst9=%0h", dst[3], dst[9]); $finish; end
endmodule
"#);
    assert!(
        text.contains("I dst3=5 dst9=b"),
        "impure helper gave wrong values:\n{text}"
    );
}
