//! IEEE 1800 §6.6 and §23.2.2.1 port-net initialization and resolution.

use xezim::simulate;

fn failures(sim: &xezim::compiler::Simulator) -> u64 {
    sim.get_signal("failures")
        .or_else(|| sim.get_signal("check_top.failures"))
        .expect("signal 'failures' not found")
        .to_u64()
        .unwrap_or(1)
}

fn bit(sim: &xezim::compiler::Simulator, name: &str) -> String {
    sim.get_signal(name)
        .unwrap_or_else(|| panic!("signal '{name}' not found"))
        .to_bin()
}

#[test]
fn root_ports_keep_pull_metadata() {
    let ansi = simulate(
        r#"
module root_cell(output tri0 value_o);
    logic enabled = 1'b1;
    assign value_o = enabled ? 1'b1 : 1'bz;
    initial #5 enabled = 1'b0;
endmodule
"#,
        20,
    )
    .expect("simulate ANSI root failed");
    assert_eq!(bit(&ansi, "value_o"), "0");

    let nonansi = simulate(
        r#"
module root_cell(value_o);
    output tri1 value_o;
    logic enabled = 1'b1;
    assign value_o = enabled ? 1'b0 : 1'bz;
    initial #5 enabled = 1'b0;
endmodule
"#,
        20,
    )
    .expect("simulate non-ANSI root failed");
    assert_eq!(bit(&nonansi, "value_o"), "1");
}

#[test]
fn explicit_port_net_types_supply_idle_values() {
    let sim = simulate(
        r#"
module pull_cell (
    output tri0    low_o,
    output tri1    high_o,
    output supply0 ground_o,
    output supply1 power_o,
    output wand    and_o,
    output wor     or_o,
    output trireg  stored_o,
    input  tri0    low_i,
    input  tri1    high_i
);
endmodule

module check_top;
    pull_cell node0 ();
    int failures;
    initial begin
        failures = 0;
        if (node0.low_o    !== 1'b0) failures++;
        if (node0.high_o   !== 1'b1) failures++;
        if (node0.ground_o !== 1'b0) failures++;
        if (node0.power_o  !== 1'b1) failures++;
        if (node0.and_o    !== 1'bz) failures++;
        if (node0.or_o     !== 1'bz) failures++;
        // §6.6.4: a trireg nothing drives is high-impedance (reference).
        if (node0.stored_o !== 1'bz) failures++;
        if (node0.low_i    !== 1'b0) failures++;
        if (node0.high_i   !== 1'b1) failures++;
    end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    assert_eq!(failures(&sim), 0);
}

#[test]
fn ansi_port_pull_returns_after_driver_releases() {
    let sim = simulate(
        r#"
module release_cell(output tri0 low_o, output tri1 high_o);
    logic enabled = 1'b1;
    assign low_o = enabled ? 1'b1 : 1'bz;
    assign high_o = enabled ? 1'b0 : 1'bz;
    initial #5 enabled = 1'b0;
endmodule

module check_top;
    wire low_seen;
    wire high_seen;
    release_cell node0 (.low_o(low_seen), .high_o(high_seen));
    int failures;
    initial begin
        failures = 0;
        #1;
        if (node0.low_o !== 1'b1) failures++;
        if (node0.high_o !== 1'b0) failures++;
        #10;
        if (node0.low_o !== 1'b0) failures++;
        if (node0.high_o !== 1'b1) failures++;
    end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    assert_eq!(failures(&sim), 0);
}

#[test]
fn nonansi_port_pull_returns_after_driver_releases() {
    let sim = simulate(
        r#"
module split_cell(value_o);
    output tri0 value_o;
    logic enabled = 1'b1;
    assign value_o = enabled ? 1'b1 : 1'bz;
    initial #5 enabled = 1'b0;
endmodule

module check_top;
    wire observed;
    split_cell node0 (observed);
    int failures;
    initial begin
        failures = 0;
        #1 if (node0.value_o !== 1'b1) failures++;
        #10 if (node0.value_o !== 1'b0) failures++;
    end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    assert_eq!(failures(&sim), 0);
}

#[test]
fn completing_net_declaration_controls_port_value() {
    let sim = simulate(
        r#"
module completion_cell(value_o);
    output value_o;
    tri1 value_o;
endmodule

module check_top;
    wire observed;
    completion_cell node0 (observed);
    int failures;
    initial begin
        failures = 0;
        if (node0.value_o !== 1'b1) failures++;
    end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    assert_eq!(failures(&sim), 0);
}

/// #1060, part 1: an `inout tri1` port bound to a top-level `wire` — when the
/// driver releases, the pull must be visible BOTH through the port formal
/// (`node0.p`) and on the outside wire (`w`). §23.3.3.7/§23.3.3.7.1 make them
/// one net, so after port substitution the pull (which §6.6.3 attaches to the
/// merged tri1 net) cannot live only on the leftover formal.
#[test]
fn inout_tri1_pull_propagates_to_actual_wire() {
    let sim = simulate(
        r#"
module sub (inout tri1 p);
    logic en = 1'b0;
    assign p = en ? 1'b0 : 1'bz;
endmodule
module top;
    wire w;
    int failures = 0;
    sub node0 (.p(w));
    initial begin
        #1;
        if (w       !== 1'b1) failures++;   // outside, undriven -> tri1 pulls high
        if (node0.p !== 1'b1) failures++;   // through the port
        node0.en = 1'b1;
        #1;
        if (w       !== 1'b0) failures++;   // driven low
        if (node0.p !== 1'b0) failures++;
    end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    assert_eq!(failures(&sim), 0);
}

/// #1060, part 2: when the actual and the inout port formal share a name
/// (`sub u (.p(p))`), the leftover formal is the same physical net and must
/// not fold a SECOND, circular reader that settles to x — the merged net
/// keeps the pull, and @(p) inside the instance wakes when the common net
/// toggles.
#[test]
fn same_named_inout_tri1_pull_keeps_net_alive_inside() {
    let sim = simulate(
        r#"
module sub (inout tri1 p);
    logic en = 1'b0;
    int   wakeups = 0;
    assign p = en ? 1'b0 : 1'bz;
    always @(p) wakeups++;
endmodule
module top;
    wire p;
    int failures = 0;
    sub node0 (.p(p));
    initial begin
        #1;
        if (p       !== 1'b1) failures++;   // outside, undriven -> pull
        if (node0.p !== 1'b1) failures++;
        node0.en = 1'b1;
        #1;
        if (p       !== 1'b0) failures++;   // driven low
        if (node0.p !== 1'b0) failures++;
        if (node0.wakeups == 0) failures++; // @(p) inside woke up
    end
endmodule
"#,
        100,
    )
    .expect("simulate failed");
    assert_eq!(failures(&sim), 0);
}
