//! §9.2.2.2.1: variables written before read inside an `always_comb` are NOT
//! part of its implicit sensitivity. The exclusion must hold at member
//! granularity too: `out = in; if (out.vld) out.f = …;` reads `out.vld`,
//! which is a read of a variable the block writes even though the string
//! doesn't equal any recorded write ("out"/"out.f"). Before the fix, the
//! surviving self-dependency re-queued the block every settle pass (the
//! pass-through write and the field write commit two values per eval) and
//! churned to the settle limit — on a real design this exhausted the slot
//! at the exact time a bus request was issued. Reference-verified values.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! `%0p` as `%p`, and no space before an associative array's closing brace.

use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_csms_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "test",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

#[test]
fn member_read_after_full_write_does_not_self_trigger() {
    let text = run(
        "rmw",
        r#"module test;
  typedef struct packed { logic vld; logic [7:0] addr; } req_t;
  req_t in_req, out_req;
  logic clk = 0; always #5 clk = ~clk;
  always_comb begin
    out_req = in_req;
    if (out_req.vld) out_req.addr = out_req.addr + 1;
  end
  initial begin
    in_req = '{1'b0, 8'h10};
    @(negedge clk) in_req = '{1'b1, 8'h20};
    repeat (2) @(posedge clk) #1 $display("T|t=%0t out_req=%p", $time, out_req);
    $finish;
  end
endmodule
"#,
    );
    assert!(!text.contains("settle limit"), "{text}");
    assert!(text.contains("T|t=16 out_req='{vld:'h1, addr:'h21}"), "{text}");
}

#[test]
fn cross_field_conditional_write_settles() {
    let text = run(
        "xfield",
        r#"module test;
  typedef struct packed { logic vld; logic [7:0] addr; } req_t;
  req_t in_req, out_req;
  logic clk = 0; always #5 clk = ~clk;
  always_comb begin
    out_req = in_req;
    if (out_req.vld) out_req.addr = 8'h55;
  end
  initial begin
    in_req = '{1'b0, 8'h10};
    @(negedge clk) in_req = '{1'b1, 8'h20};
    repeat (2) @(posedge clk) #1 $display("T|t=%0t out_req=%p", $time, out_req);
    $finish;
  end
endmodule
"#,
    );
    assert!(!text.contains("settle limit"), "{text}");
    assert!(text.contains("T|t=16 out_req='{vld:'h1, addr:'h55}"), "{text}");
}

#[test]
fn block_still_refires_on_real_input_change() {
    // The exclusion must not eat the block's genuine inputs: a second
    // change of in_req must recompute out_req.
    let text = run(
        "refire",
        r#"module test;
  typedef struct packed { logic vld; logic [7:0] addr; } req_t;
  req_t in_req, out_req;
  logic clk = 0; always #5 clk = ~clk;
  always_comb begin
    out_req = in_req;
    if (out_req.vld) out_req.addr = out_req.addr + 1;
  end
  initial begin
    in_req = '{1'b1, 8'h20};
    @(negedge clk) in_req = '{1'b1, 8'h40};
    @(posedge clk) #1 $display("T|t=%0t out_req=%p", $time, out_req);
    $finish;
  end
endmodule
"#,
    );
    assert!(!text.contains("settle limit"), "{text}");
    assert!(text.contains("out_req='{vld:'h1, addr:'h41}"), "{text}");
}
