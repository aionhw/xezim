//! IEEE 1800-2023 §3.14.2.2 and §20.3.1–§20.3.3: time queries
//! use the time unit of the module that invokes them.

fn check(source: &str, expected: &[&str]) {
    let sim = xezim::simulate(source, 10_000).expect("simulate");
    let mut observed: Vec<_> = sim
        .output
        .iter()
        .filter_map(|line| line.message.trim().strip_prefix("CHECK "))
        .collect();
    observed.sort_unstable();
    assert_eq!(observed, expected);
}

#[test]
fn realtime_in_top_assign_woken_by_child() {
    check(
        r#"
module child(output bit ready);
  timeunit 1ps/1ps;
  initial #3000 ready = 1;
endmodule

module top;
  timeunit 1ns/1ps;
  wire ready;
  child u(ready);
  real observed;
  assign observed = ready ? $realtime : 0.0;

  initial begin
    #4;
    $display("CHECK observed=%g", observed);
    $finish;
  end
endmodule
"#,
        &["observed=3"],
    );
}

// Both module instances have the same readers. The child wakes the top at
// 3 ns and the top wakes the child at 4 ns. Read after either #1 delay expires.
const ASSIGN_READERS: &str = r#"
  time stamp_time;
  int unsigned stamp_stime;
  real stamp_real, stamp_function;
  wire [63:0] stamp_delayed;

  function automatic real sample_time(input bit ready);
    return ready ? $realtime : 0.0;
  endfunction

  assign stamp_time = ready ? $time : 0;
  assign stamp_stime = ready ? $stime : 0;
  assign stamp_real = ready ? $realtime : 0.0;
  assign stamp_function = sample_time(ready);
  assign #1 stamp_delayed = ready ? $time : 0;

  initial begin
    #5500ps;
    $display("CHECK %m time=%0d stime=%0d realtime=%g function=%g delayed=%0d",
             stamp_time, stamp_stime, stamp_real, stamp_function, stamp_delayed);
  end
"#;

const PROCEDURAL_READERS: &str = r#"
  real stamp_comb, stamp_star, stamp_initial;
  always_comb if (ready) stamp_comb = $realtime;
  always @* if (ready) stamp_star = $realtime;
  initial begin
    @(posedge ready);
    stamp_initial = $realtime;
  end

  initial begin
    #5500ps;
    $display("CHECK %m always_comb=%g always_star=%g initial=%g",
             stamp_comb, stamp_star, stamp_initial);
  end
"#;

fn mixed_units(top_unit: &str, child_unit: &str, readers: &str) -> String {
    format!(
        r#"
module child(input wire ready, output bit child_ready);
  timeunit {child_unit}/1ps;
  initial #3ns child_ready = 1;
{readers}
endmodule

module top;
  timeunit {top_unit}/1ps;
  bit top_ready;
  wire ready;
  child u(top_ready, ready);
  initial #4ns top_ready = 1;
{readers}
  initial #6ns $finish;
endmodule
"#
    )
}

#[test]
fn continuous_assignments_in_ns_top_and_ps_child() {
    check(
        &mixed_units("1ns", "1ps", ASSIGN_READERS),
        &[
            "top time=3 stime=3 realtime=3 function=3 delayed=3",
            "top.u time=4000 stime=4000 realtime=4000 function=4000 delayed=4000",
        ],
    );
}

#[test]
fn continuous_assignments_in_ps_top_and_ns_child() {
    check(
        &mixed_units("1ps", "1ns", ASSIGN_READERS),
        &[
            "top time=3000 stime=3000 realtime=3000 function=3000 delayed=3000",
            "top.u time=4 stime=4 realtime=4 function=4 delayed=4",
        ],
    );
}

fn resolver(top_unit: &str, child_unit: &str) -> String {
    format!(
        r#"
module child(output bit ready);
  timeunit {child_unit}/1ps;
  initial #3ns ready = 1;
endmodule

module top;
  timeunit {top_unit}/1ps;
  function automatic real resolve(input real drivers[]);
    real sum = 0.0;
    foreach (drivers[i]) sum += drivers[i];
    return sum == 0.0 ? 0.0 : $realtime;
  endfunction
  nettype real resolved_real with resolve;
  wire ready;
  child u(ready);
  resolved_real observed;
  assign observed = ready ? 1.0 : 0.0;

  initial begin
    #5ns;
    $display("CHECK resolver=%g", observed);
    $finish;
  end
endmodule
"#
    )
}

#[test]
fn nettype_resolver_in_ns_top() {
    check(&resolver("1ns", "1ps"), &["resolver=3"]);
}

#[test]
fn nettype_resolver_in_ps_top() {
    check(&resolver("1ps", "1ns"), &["resolver=3000"]);
}

#[test]
fn procedural_readers_in_ns_top_and_ps_child() {
    check(
        &mixed_units("1ns", "1ps", PROCEDURAL_READERS),
        &[
            "top always_comb=3 always_star=3 initial=3",
            "top.u always_comb=4000 always_star=4000 initial=4000",
        ],
    );
}

#[test]
fn procedural_readers_in_ps_top_and_ns_child() {
    check(
        &mixed_units("1ps", "1ns", PROCEDURAL_READERS),
        &[
            "top always_comb=3000 always_star=3000 initial=3000",
            "top.u always_comb=4 always_star=4 initial=4",
        ],
    );
}

/// §23.6: the time call is written in top. The target lives in the child.
#[test]
fn hierarchical_lhs_keeps_the_writing_modules_time_unit() {
    check(
        r#"
module child;
  timeunit 1ns/1ps;
  wire [63:0] observed;
endmodule

module top;
  timeunit 1ps/1ps;
  bit ready;
  child u();
  assign u.observed = ready ? $time : 0;

  initial begin
    #3000 ready = 1;
    #1000;
    $display("CHECK observed=%0d", u.observed);
    $finish;
  end
endmodule
"#,
        &["observed=3000"],
    );
}

/// §23.3.3: the input-port expression belongs to the nested parent.
#[test]
fn nested_port_actual_keeps_the_parents_time_unit() {
    check(
        r#"
module sink(input wire [63:0] stamp);
  timeunit 1ns/1ps;
endmodule

module parent;
  timeunit 1ps/1ps;
  bit ready;
  sink u(ready ? $time : 0);
  initial #3000 ready = 1;
endmodule

module top;
  timeunit 1ns/1ps;
  parent p();

  initial begin
    #5;
    $display("CHECK port_actual=%0d", p.u.stamp);
    $finish;
  end
endmodule
"#,
        &["port_actual=3000"],
    );
}
