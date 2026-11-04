//! §9.2/§9.4.2: a write made by a process CONTINUATION resumed inline in the
//! active region (blocks-first mode: `@(posedge clk) sig = …;`) must re-run
//! edge detection for that signal, exactly like a write made inside an edge
//! block. Before the fix, continuation writes were not recorded for the
//! rescan drain (`in_edge_block` was already cleared), the next snapshot
//! refresh swallowed the change, and an AnyEdge-routed comb block sensitive
//! to the signal NEVER fired again — TB-driven data silently stopped flowing
//! into `@*` logic whose body reads what it writes (the self-referential
//! shape is routed through the edge path). The legacy scheduler
//! (XEZIM_WAITERS_FIRST=1 XEZIM_ACTIVE_REGION=0) was unaffected, matching a
//! customer report of "worked in an earlier release". All expected values
//! reference-verified.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! `%0p` as `%p`, and no space before an associative array's closing brace.

use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_wcaw_{}_{}", name, std::process::id()));
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
fn waiter_continuation_write_wakes_anyedge_comb_block() {
    // The @* body writes `o` then reads `o.vld`: self-referential, so it is
    // routed through the EDGE path with inferred AnyEdge sensitivity on
    // `in_q`. `in_q` is driven from `@(negedge clk)` continuations.
    let text = run(
        "cont_wake",
        r#"module test;
  typedef struct packed { logic vld; logic [7:0] addr; } req_t;
  req_t in_q, o;
  logic clk = 0; always #5 clk = ~clk;
  always @(*) begin o = in_q; if (o.vld) o.addr = o.addr + 1; end
  initial begin
    in_q = '{1'b0, 8'h10};
    @(negedge clk) in_q = '{1'b1, 8'h20};
    #1 $display("T|t=%0t o=%p", $time, o);
    #6 $display("T|t=%0t o=%p", $time, o);
    #10 $display("T|t=%0t o=%p", $time, o);
    @(negedge clk) in_q = '{1'b1, 8'h30};
    #1 $display("T|t=%0t o=%p", $time, o);
    #2 $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("T|t=11 o='{vld:'h1, addr:'h21}"), "{text}");
    assert!(text.contains("T|t=17 o='{vld:'h1, addr:'h21}"), "{text}");
    assert!(text.contains("T|t=27 o='{vld:'h1, addr:'h21}"), "{text}");
    assert!(text.contains("T|t=31 o='{vld:'h1, addr:'h31}"), "{text}");
}

#[test]
fn zero_delay_continuation_write_wakes_anyedge_comb_block() {
    // §4.4.2.3/§9.2: `@(negedge clk) begin #0 sig = …; end` parks the rest
    // of the process in the Inactive-region queue; its write runs after the
    // slot's edge detection. Before the fix, drain_inactive_pre_nba neither
    // recorded the write for the rescan drain nor ran another detect (it
    // broke early with no NBAs pending), so the AnyEdge block never woke —
    // and the nested-#0 path also needed the in_edge_cont flag to
    // save/restore rather than clear. Reference-verified.
    let text = run(
        "zero_delay_wake",
        r#"module test;
  typedef struct packed { logic vld; logic [7:0] addr; } req_t;
  req_t in_q, o;
  logic clk = 0; always #5 clk = ~clk;
  always @(*) begin o = in_q; if (o.vld) o.addr = o.addr + 1; end
  initial begin
    in_q = '{1'b0, 8'h10};
    @(negedge clk) begin #0 in_q = '{1'b1, 8'h20}; end
    #1 $display("T|t=%0t o=%p", $time, o);
    #10 $display("T|t=%0t o=%p", $time, o);
    #2 $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("T|t=11 o='{vld:'h1, addr:'h21}"), "{text}");
    assert!(text.contains("T|t=21 o='{vld:'h1, addr:'h21}"), "{text}");
}

#[test]
fn delay_driven_input_still_works() {
    // Control: the same shape driven by #delay writes (never broken).
    let text = run(
        "delay_ctl",
        r#"module test;
  typedef struct packed { logic vld; logic [7:0] addr; } req_t;
  req_t in_q, o;
  always @(*) begin o = in_q; if (o.vld) o.addr = o.addr + 1; end
  initial begin
    in_q = '{1'b0, 8'h10};
    #10 in_q = '{1'b1, 8'h20};
    #1 $display("T|t=%0t o=%p", $time, o);
    #2 $finish;
  end
endmodule
"#,
    );
    assert!(text.contains("T|t=11 o='{vld:'h1, addr:'h21}"), "{text}");
}

#[test]
fn explicit_anyedge_self_ref_counter_fires_per_change() {
    // Control: explicit @(a) self-referential counter, #delay-driven —
    // fires at t=0 plus once per change of `a` (reference: cnt=3).
    let text = run(
        "cnt_ctl",
        r#"module test;
  reg [7:0] a; reg [7:0] cnt;
  initial begin
    a = 8'h10; cnt = 0;
    #10 a = 8'h20;
    #10 a = 8'h21;
    #5 $display("T|cnt=%0d", cnt); $finish;
  end
  always @(a) cnt = cnt + 1;
endmodule
"#,
    );
    assert!(text.contains("T|cnt=3"), "{text}");
}
