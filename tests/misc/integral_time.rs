//! IEEE 1800-2023 §20.3.1–§20.3.3: simulation time in the calling scope's
//! unit. The fixtures use explicit time scopes (§3.14.2.2), integer delays
//! (§9.4.1) and decimal/time display fields (§21.2.1, §20.4.3).

fn check(src: &str, expected: &[&str]) {
    let sim = xezim::simulate(src, 10_000_000_000_000).expect("simulate failed");
    let output: Vec<_> = sim
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(output, expected);
}

#[test]
fn equal_unit_preserves_ticks_and_stime_wrap() {
    // §10.3: the continuous assignment captures the odd tick on the gate change.
    // The read one tick later occurs after the net update.
    // §8.6, §13.3, §13.4: method, task and function reads use their scope's unit.
    check(
        r#"
module top;
  timeunit 1fs; timeprecision 1fs;
  bit gate;
  wire [63:0] captured_time;
  wire [31:0] captured_stime;
  assign captured_time = gate ? $time : 0;
  assign captured_stime = gate ? $stime : 0;
  function automatic time ticks();
    return $time();
  endfunction
  task automatic show_ticks();
    $display("task %0d %0d", $time, $stime);
  endtask
  class reader;
    function int unsigned ticks();
      return $stime();
    endfunction
  endclass
  reader obj;
  initial begin
    obj = new;
    $timeformat(-15, 0, "", 0);
    #(64'd4294967297);
    $display("wrap %0d %0d", $time, $stime);
    #(64'd9007194959773694);
    $display("before %0d %0d", $time, $stime);
    #1;
    $display("boundary %0d %0d", $time, $stime);
    #1;
    gate = 1;
    $display("odd %0d %0d", $time, $stime);
    $display("calls %0d %0d", ticks(), obj.ticks());
    show_ticks();
    $display("format %0t %0t", $time, $stime);
    #1;
    $display("capture %0d %0d", captured_time, captured_stime);
    $display("after %0d %0d", $time, $stime);
    #(64'd9214364837600033792);
    #1021;
    $display("large %0d %0d", $time, $stime);
    $finish;
  end
endmodule
"#,
        &[
            "wrap 4294967297 1",
            "before 9007199254740991 4294967295",
            "boundary 9007199254740992 0",
            "odd 9007199254740993 1",
            "calls 9007199254740993 1",
            "task 9007199254740993 1",
            "format 9007199254740993 1",
            "capture 9007199254740993 1",
            "after 9007199254740994 2",
            "large 9223372036854775807 4294967295",
        ],
    );
}

#[test]
fn coarser_unit_rounds_at_the_integer_tick_midpoint() {
    // §6.12.1, §20.3.1: 0.5 rounds up. At large times, the last tick
    // below the midpoint still rounds down. §20.3.3 keeps a real 0.5.
    check(
        r#"
module top;
  timeunit 1ps; timeprecision 1fs;
  initial begin
    $timeformat(-12, 0, "", 0);
    #499fs;
    $display("small below %0d %0d", $time, $stime);
    #1fs;
    $display("small tie %0d %0d %0.1f", $time, $stime, $realtime);
    #1fs;
    $display("small above %0d %0d", $time, $stime);
    repeat (10) #(64'd900719925474);
    #998fs;
    $display("large below %0d %0d %0t %0t", $time, $stime, $time, $stime);
    #1fs;
    $display("large tie %0d %0d", $time, $stime);
    #1fs;
    $display("large above %0d %0d", $time, $stime);
    $finish;
  end
endmodule
"#,
        &[
            "small below 0 0",
            "small tie 1 1 0.5",
            "small above 1 1",
            "large below 9007199254741 652835029 9007199254741 652835029",
            "large tie 9007199254742 652835030",
            "large above 9007199254742 652835030",
        ],
    );
}

#[test]
fn package_function_reports_exact_time_in_its_own_unit() {
    // §3.14.2.3, §26.3: the package function uses the package's time unit.
    check(
        r#"
package clocks;
  timeunit 1fs; timeprecision 1fs;
  function automatic time ticks();
    return $time;
  endfunction
endpackage
module top;
  timeunit 1ps; timeprecision 1fs;
  initial begin
    repeat (10) #(64'd900719925474);
    #993fs;
    $display("package %0d local %0d", clocks::ticks(), $time);
    $finish;
  end
endmodule
"#,
        &["package 9007199254740993 local 9007199254741"],
    );
}

#[test]
fn time_format_scales_direct_and_stored_integral_values_exactly() {
    // §20.4.3: 9223372035620195 ps is exactly 922337203562019.5 in 10 ps
    // display units. One fractional digit therefore needs no rounding.
    check(
        r#"
module top;
  timeunit 1ps; timeprecision 1fs;
  time saved;
  logic [127:0] wide;
  initial begin
    $timeformat(-11, 1, "", 0);
    repeat (18446) #(64'd500000000000);
    #(64'd372035620194);
    #500fs;
    saved = $time;
    wide = $time;
    $display("scaled %0d %0t %0t %0t", $time, $time, saved, wide);
    $finish;
  end
endmodule
"#,
        &["scaled 9223372035620195 922337203562019.5 922337203562019.5 922337203562019.5"],
    );
}
