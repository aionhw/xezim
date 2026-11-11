//! IEEE 1800-2023 §9.6.2, §12.8: a `disable`, `break` or `continue` that
//! runs in a `forever` body before the body's delay ends the iteration
//! there, and `disable` of a block stops that block when it is the
//! process's own top-level block (`initial begin : blk`), the body of an
//! `always`, or a named `forever` body. Expected values are the reference
//! simulator's.

use xezim::simulate;

/// The `T|` lines, in output order.
fn t_lines(body: &str) -> Vec<String> {
    let src = format!(
        "module t;\n  int n, x, y;\n{body}\n  initial #50 begin $display(\"T|end n=%0d x=%0d y=%0d\", n, x, y); $finish; end\nendmodule\n"
    );
    let sim = simulate(&src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn a_disable_before_the_delay_in_a_forever_ends_the_enclosing_block() {
    let o = t_lines(
        r#"
  initial begin
    begin : lbl
      forever begin n++; if (n == 3) disable lbl; #1; end
    end
    x = 1;
    $display("T|out n=%0d t=%0t", n, $time);
  end
"#,
    );
    assert_eq!(o, ["T|out n=3 t=2", "T|end n=3 x=1 y=0"]);
}

#[test]
fn a_break_or_continue_before_the_delay_in_a_forever_is_honoured() {
    let o = t_lines(
        r#"
  initial begin
    forever begin n++; if (n == 3) break; #1; end
    $display("T|break n=%0d t=%0t", n, $time);
    forever begin y++; if (y < 3) continue; x++; if (y == 5) break; #1; end
    $display("T|continue y=%0d x=%0d t=%0t", y, x, $time);
  end
"#,
    );
    assert_eq!(
        o,
        [
            "T|break n=3 t=2",
            "T|continue y=5 x=3 t=4",
            "T|end n=3 x=3 y=5"
        ]
    );
}

#[test]
fn a_disable_of_the_processs_own_top_level_block_ends_the_process() {
    let o = t_lines(
        r#"
  initial begin : blk
    forever begin #1 n++; if (n == 3) disable blk; end
  end
  initial begin : blk2
    begin : inner
      forever begin #1 x++; if (x == 2) begin : deep disable blk2; y = 9; end end
    end
    y = 7;
  end
"#,
    );
    assert_eq!(o, ["T|end n=3 x=2 y=0"]);
}

#[test]
fn a_disable_of_a_timing_free_top_level_block_skips_the_rest() {
    let o = t_lines(
        r#"
  initial begin : blk n++; disable blk; n++; end
  initial begin : blk2 #1 x++; disable blk2; x++; end
"#,
    );
    assert_eq!(o, ["T|end n=1 x=1 y=0"]);
}

#[test]
fn a_disable_of_an_always_label_ends_only_that_iteration() {
    let o = t_lines(
        r#"
  always begin : ab #1 n++; if (n == 2) disable ab; x++; #1 y++; end
"#,
    );
    assert_eq!(o, ["T|end n=25 x=24 y=24"]);
}

#[test]
fn a_named_forever_body_is_a_block_for_disable_and_percent_m() {
    let o = t_lines(
        r#"
  initial begin
    forever begin : it #1 n++; if (n < 3) disable it; x++; if (n == 5) break; end
    $display("T|out n=%0d x=%0d t=%0t", n, x, $time);
  end
  always begin : ab #2 y++; if (y == 1) $display("T|m=%m"); end
"#,
    );
    assert_eq!(o, ["T|m=t.ab", "T|out n=5 x=3 t=5", "T|end n=5 x=3 y=24"]);
}

#[test]
fn disables_that_already_stopped_the_right_block_keep_working() {
    let o = t_lines(
        r#"
  task tk; forever begin #1 y++; if (y == 3) disable tk; end endtask
  initial begin
    begin : a forever begin #1; n++; if (n == 3) disable a; x++; end end
    $display("T|a n=%0d x=%0d t=%0t", n, x, $time);
    begin : b n++; disable b; n++; end
    begin : c forever begin n++; if (n == 6) disable c; end end
    tk;
    $display("T|b n=%0d y=%0d t=%0t", n, y, $time);
  end
"#,
    );
    assert_eq!(
        o,
        ["T|a n=3 x=2 t=3", "T|b n=6 y=3 t=6", "T|end n=6 x=2 y=3"]
    );
}
