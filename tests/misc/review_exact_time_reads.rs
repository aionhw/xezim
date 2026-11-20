//! Follow-up to #1329: integral delays (§9.4.1, §3.14.1), the severity
//! `Time:` line (§20.10), `%t` of `$realtime` (§20.3.3, §20.4.3) and time
//! reads in package subroutines (§3.14.2.3, §20.3.2) keep every tick above
//! 2^53 and report in the declaring scope's unit.

fn messages(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 10_000_000_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .collect()
}

/// §9.4.1, §3.14.1: an integral delay of 2^53 + 1 ticks wakes at that tick,
/// whatever holds the count. Converting it through binary64 woke one tick
/// early. The reference simulator converts these delays the same way; the
/// LRM gives the delay as the integer count.
#[test]
fn integral_delays_above_2_53_ticks_are_exact() {
    let src = r#"
package pk;
  timeunit 1fs; timeprecision 1fs;
  localparam time PD = 64'd9007199254740993;
  time pv = 64'd9007199254740993;
endpackage
class C; time d = 64'd9007199254740993; endclass
module top;
  timeunit 1fs; timeprecision 1fs;
  parameter time P = 64'd9007199254740993;
  localparam time L = 64'd9007199254740993;
  time tv = 64'd9007199254740993;
  time da[] = '{64'd9007199254740993};
  time q[$] = '{64'd9007199254740993};
  time aa[int];
  C c = new;
  logic r = 0;
  initial begin
    aa[0] = 64'd9007199254740993;
    fork
      begin #(64'd9007199254740993); $display("sized %0d", $time); end
      begin #P; $display("param %0d", $time); end
      begin #L; $display("lparam %0d", $time); end
      begin #(pk::PD); $display("pkg_lparam %0d", $time); end
      begin #tv; $display("var %0d", $time); end
      begin #(c.d); $display("class %0d", $time); end
      begin #(da[0]); $display("dyn %0d", $time); end
      begin #(q[0]); $display("queue %0d", $time); end
      begin #(aa[0]); $display("assoc %0d", $time); end
      begin #(pk::pv); $display("pkg_var %0d", $time); end
      begin #($unsigned(tv)); $display("unsigned %0d", $time); end
      begin #(tv + 0); $display("plus0 %0d", $time); end
      begin fork #tv; join $display("fork %0d", $time); end
      r <= #tv 1'b1;
    join_none
    #(64'd9007199254740995) $finish;
  end
  always @(posedge r) $display("nba %0d", $time);
endmodule
"#;
    let out = messages(src);
    for form in [
        "sized",
        "param",
        "lparam",
        "pkg_lparam",
        "var",
        "class",
        "dyn",
        "queue",
        "assoc",
        "pkg_var",
        "unsigned",
        "plus0",
        "fork",
        "nba",
    ] {
        let expected = format!("{form} 9007199254740993");
        assert!(
            out.iter().any(|m| *m == expected),
            "missing {expected:?}: {out:#?}"
        );
    }
}

/// §20.10: the severity `Time:` line, and §20.3.3, §20.4.3: `%t` of
/// `$realtime` in a coarser unit, scale the tick count itself, so the odd
/// tick at 2^53 + 1 fs survives.
#[test]
fn severity_time_line_and_realtime_display_keep_every_tick() {
    let src = r#"
module top;
  timeunit 1ps; timeprecision 1fs;
  initial begin
    repeat (10) #(64'd900719925474);
    #993fs;
    $info("odd tick");
    $timeformat(-15, 0, "", 0);
    $display("realtime %0t", $realtime);
    $timeformat(-12, 3, " ps", 0);
    $display("realtime ps %0t", $realtime);
    $finish;
  end
endmodule
"#;
    let out = messages(src);
    let expected = [
        "** Info: odd tick",
        "   Time: 9007199254740993  Scope: top",
        "realtime 9007199254740993",
        "realtime ps 9007199254740.993 ps",
    ];
    for line in expected {
        assert!(out.iter().any(|m| m == line), "missing {line:?}: {out:#?}");
    }
}

/// §3.14.2.3: `%t` of `$time`, `$stime` and `$realtime` read in a package
/// function or a package class method scales from the package's 1 ns unit,
/// not the calling module's 1 ps one.
#[test]
fn package_subroutine_time_reads_display_from_the_package_unit() {
    let src = r#"
package pk;
  timeunit 1ns; timeprecision 1ps;
  function automatic void show();
    $timeformat(-9, 3, " ns", 0);
    $display("pkg %0t / %0t / %0t", $time, $stime, $realtime);
  endfunction
  class K;
    static function void show();
      $display("cls %0t / %0t / %0t", $time, $stime, $realtime);
    endfunction
  endclass
endpackage
module top;
  timeunit 1ps; timeprecision 1ps;
  initial begin
    #2500;
    pk::show();
    pk::K::show();
    $display("mod %0t / %0t / %0t", $time, $stime, $realtime);
    $finish;
  end
endmodule
"#;
    let out = messages(src);
    for line in [
        "pkg 3.000 ns / 3.000 ns / 2.500 ns",
        "cls 3.000 ns / 3.000 ns / 2.500 ns",
        "mod 2.500 ns / 2.500 ns / 2.500 ns",
    ] {
        assert!(out.iter().any(|m| m == line), "missing {line:?}: {out:#?}");
    }
}

/// §20.3.2: `$stime` read in a package function keeps the low 32 bits, as
/// in a module. §21.2.3: a `$monitor` started from a package task prints on
/// a change of its variable argument, not on every advance of its `$time`
/// argument.
#[test]
fn package_stime_wraps_and_package_monitor_ignores_time() {
    let src = r#"
package pk;
  timeunit 1fs; timeprecision 1fs;
  int pv = 0;
  function automatic time st(); return time'($stime); endfunction
  function automatic time st_raw(); return $stime; endfunction
  task watch();
    $monitor("monitor t=%0t v=%0d", $time, pv);
  endtask
endpackage
module top;
  timeunit 1fs; timeprecision 1fs;
  initial begin
    pk::watch();
    #(64'd4294967297);
    $display("pkg stime %0d %0d", pk::st(), pk::st_raw());
    #3 pk::pv = 1;
    #1;
    #1;
    #4 pk::pv = 2;
    #5 $finish;
  end
endmodule
"#;
    assert_eq!(
        messages(src),
        [
            "monitor t=0 v=0",
            "pkg stime 1 1",
            "monitor t=4294967300 v=1",
            "monitor t=4294967306 v=2",
        ]
    );
}
