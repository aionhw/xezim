//! IEEE 1800-2023 §6.5, §6.6.2, §6.6.6, §28.12 and Clause 29.

#[test]
fn primitive_and_net_strengths_resolve_by_value_and_strength() {
    let src = r#"
primitive buffer(out, a);
  output out; input a;
  table
    0 : 0;
    1 : 1;
  endtable
endprimitive
module leaf;
  wire isolated;
  buf (strong0, highz1) g(isolated, 1'b1);
  initial #1 $display("nested %b %v", isolated, isolated);
endmodule
module top;
  leaf u();
  wire gate_net, single_z, pulled, udp_net;
  supply0 power, supply_array[2];
  supply1 supply_ones[2];
  wand wired_and;
  wor wired_or;
  wand differing_strengths;
  buf (weak0, weak1) gate0(gate_net, 1'b0);
  buf (strong0, strong1) gate1(gate_net, 1'b1);
  buf (strong0, highz1) isolated(single_z, 1'b1);
  pullup (strong1) up(pulled);
  pulldown (weak0) down(pulled);
  buffer (weak0, weak1) udp0(udp_net, 1'b0);
  buffer (strong0, strong1) udp1(udp_net, 1'b1);
  assign (weak0, weak1) power = 1'b1;
  assign (pull0, pull1) wired_and = 1'b0;
  assign (pull0, pull1) wired_and = 1'b1;
  assign (pull0, pull1) wired_or = 1'b0;
  assign (pull0, pull1) wired_or = 1'b1;
  assign (weak0, weak1) differing_strengths = 1'b0;
  assign (strong0, strong1) differing_strengths = 1'b1;
  initial #1 begin
    $display("R %v %v %v %v %v %v %v %v %b%b", gate_net, single_z,
             pulled, udp_net, power, wired_and, wired_or, differing_strengths,
             supply_array[1], supply_ones[0]);
    $finish;
  end
endmodule
"#;
    let sim = xezim::simulate(src, 100).expect("simulation");
    assert!(
        sim.output
            .iter()
            .any(|out| out.message.trim() == "R St1 HiZ St1 St1 Su0 Pu0 Pu1 St1 01"),
        "{:?}",
        sim.output
    );
    assert!(
        sim.output
            .iter()
            .any(|out| out.message.trim() == "nested z HiZ")
    );
}

#[test]
fn uwire_rejects_overlapping_drivers_and_pass_switches() {
    for (body, diagnostic) in [
        ("uwire w; assign w = 0; assign w = 1;", "multiple drivers"),
        (
            "uwire [2:0] w; assign w[2:1] = 0; assign w[1:0] = 1;",
            "multiple drivers",
        ),
        ("uwire a, b; tran t(a, b);", "bidirectional switch"),
    ] {
        let source = format!("module top; {body} endmodule");
        let err = xezim::simulate(&source, 100)
            .err()
            .expect("invalid uwire accepted");
        assert!(err.contains(diagnostic), "{body}: {err}");
    }
    let sim = xezim::simulate(
        "module top; uwire [1:0] w; assign w[0] = 0; assign w[1] = 1; initial #1 $display(\"%b\", w); endmodule",
        100,
    )
    .expect("disjoint uwire drivers");
    assert_eq!(sim.output[0].message.trim(), "10");
}

#[test]
fn procedural_net_write_check_respects_lvalues_and_local_shadowing() {
    for body in [
        "wire [3:0] w; bit v; initial {w[0], v} = 2'b11;",
        "wire [3:0] w; initial w++;",
        "wire w; function void f(); w = 1; endfunction",
        "wire w; final w = 1;",
        "nettype real R; R w; initial w = 1.5;",
    ] {
        let source = format!("module top; {body} endmodule");
        let err = xezim::simulate(&source, 100)
            .err()
            .expect("net write accepted");
        assert!(
            err.contains("target of a procedural assignment"),
            "{body}: {err}"
        );
    }
    let err = xezim::simulate(
        "module top(output wire logic w); initial w = 1; endmodule",
        100,
    )
    .err()
    .expect("explicit output net write accepted");
    assert!(err.contains("target of a procedural assignment"), "{err}");
    let valid = r#"
module top;
  wire w;
  initial begin
    begin logic w; w = 1; $display("%b", w); end
    force w = 1;
    release w;
  end
endmodule
"#;
    let sim = xezim::simulate(valid, 100).expect("local shadow and force");
    assert_eq!(sim.output[0].message.trim(), "1");
}

#[test]
fn declared_initializer_counts_as_a_procedural_driver() {
    let err = xezim::simulate("module top; logic v = 0; assign v = 1; endmodule", 100)
        .err()
        .expect("mixed drivers accepted");
    assert!(
        err.contains("both continuous and procedural drivers"),
        "{err}"
    );
}
