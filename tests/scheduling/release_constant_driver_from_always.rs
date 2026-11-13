//! §10.6.2: `release` of a net returns it to the value of its continuous
//! drivers, also when the driver reads no signal (`wire w = 4'h5;`,
//! `assign w = 4'h6;`) and the release runs in an `always` block. There the
//! re-drive is deferred to the next settle, which only re-runs drivers whose
//! inputs changed, so a constant driver was never re-run and the net kept the
//! forced value. A release from `initial` re-drove it at once.
//! Expected values were checked on the reference simulator.

fn run(src: &str) -> Vec<String> {
    let result = xezim::simulate(src, 100_000).unwrap();
    result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn release_from_event_control_always_redrives_constant_drivers() {
    let lines = run(r#"
module t;
  wire [3:0] a = 4'h5;
  wire [3:0] b; assign b = 4'h6;
  logic s = 1'b0;
  initial begin force a = 4'h0; force b = 4'h0; end
  always @(s) begin release a; release b; end
  initial begin
    #1 s = 1'b1;
    #1 $display("R released a=%h b=%h", a, b);
  end
endmodule
"#);
    assert_eq!(lines, ["R released a=5 b=6"]);
}

#[test]
fn release_from_always_comb_and_star_redrives_constant_drivers() {
    let lines = run(r#"
module t;
  wire [3:0] a = 4'h5;
  wire [3:0] b; assign b = 4'h6;
  logic s = 1'b0, r = 1'b0;
  initial begin force a = 4'h0; force b = 4'h0; end
  always_comb if (s) release a;
  always @* if (r) release b;
  initial begin
    #1 s = 1'b1; r = 1'b1;
    #1 $display("R released a=%h b=%h", a, b);
  end
endmodule
"#);
    assert_eq!(lines, ["R released a=5 b=6"]);
}

#[test]
fn release_from_edge_always_redrives_constant_driver() {
    let lines = run(r#"
module t;
  wire [3:0] a = 4'h5;
  logic clk = 1'b0;
  initial force a = 4'h0;
  always @(posedge clk) release a;
  initial begin
    #1 clk = 1'b1;
    #1 $display("R released a=%h", a);
  end
endmodule
"#);
    assert_eq!(lines, ["R released a=5"]);
}

/// The released net is a sub-module output with a constant driver, read
/// back through the port: the shape of a UPF power-up (§10.6.2, §23.3.3).
#[test]
fn release_of_submodule_constant_output_reaches_the_parent() {
    let lines = run(r#"
module c(output wire [3:0] q);
  assign q = 4'h9;
endmodule
module t;
  wire [3:0] y;
  logic on = 1'b1;
  c u(.q(y));
  always @(on) if (on) release u.q; else force u.q = 4'h0;
  initial begin
    #1 on = 1'b0;
    #1 $display("R down u.q=%h y=%h", u.q, y);
    on = 1'b1;
    #1 $display("R up u.q=%h y=%h", u.q, y);
    on = 1'b0;
    #1 $display("R down2 u.q=%h y=%h", u.q, y);
    on = 1'b1;
    #1 $display("R up2 u.q=%h y=%h", u.q, y);
  end
endmodule
"#);
    assert_eq!(
        lines,
        [
            "R down u.q=0 y=0",
            "R up u.q=9 y=9",
            "R down2 u.q=0 y=0",
            "R up2 u.q=9 y=9"
        ]
    );
}

/// Guard: a release from `initial` already re-drove a constant driver.
#[test]
fn release_from_initial_guard() {
    let lines = run(r#"
module t;
  wire [3:0] a = 4'h5;
  logic s = 1'b0;
  initial force a = 4'h0;
  initial begin @(s); release a; end
  initial begin
    #1 s = 1'b1;
    #1 $display("R released a=%h", a);
  end
endmodule
"#);
    assert_eq!(lines, ["R released a=5"]);
}

/// Guard: a released variable keeps the forced value until the next
/// procedural assignment (§10.6.2).
#[test]
fn release_of_variable_from_always_keeps_forced_value_guard() {
    let lines = run(r#"
module t;
  logic [3:0] v = 4'h3;
  logic s = 1'b0;
  initial force v = 4'h0;
  always @(s) release v;
  initial begin
    #1 s = 1'b1;
    #1 $display("R released v=%h", v);
    v = 4'h7;
    #1 $display("R assigned v=%h", v);
  end
endmodule
"#);
    assert_eq!(lines, ["R released v=0", "R assigned v=7"]);
}

/// §21.2.1.7: evaluating a released child's driver preserves the caller's %m.
#[test]
fn release_keeps_the_calling_procedure_scope() {
    let lines = run(r#"
module leaf; wire [3:0] n=4'h9; endmodule
module top;
  leaf u(); bit trigger;
  initial force u.n=0;
  always @(trigger) begin : releasing
    string before_scope, after_scope;
    before_scope=$sformatf("%m");
    release u.n;
    after_scope=$sformatf("%m");
    if (before_scope != after_scope) $fatal(1,"release changed scope");
  end
  initial begin #1; trigger=1; #1; $display("R scope n=%h",u.n); end
endmodule
"#);
    assert_eq!(lines, ["R scope n=9"]);
}
