//! IEEE 1800-2023 §7.2.1, §7.4.5, §23.10: a select into a packed struct array
//! declared in an instantiated module (`par[1].d`, with `s_t [N-1:0] par`
//! and a member `logic [W-1:0] d`) is sized by that instance's own
//! parameters. The interpreted read finds them through the instance's scope.
//! Finding one scope's parameters scanned the whole design-wide table, which
//! holds every instance's parameters, so the time-0 settle of a design with
//! many such instances was quadratic in its size: a million continuous
//! assignments did not settle in 45 minutes (#1563). The
//! deterministic work counter `decl_param_rows` must grow with the number of
//! instances, not with instances times the table. Each leaf's `W` differs
//! from its neighbour's and from the top's `W`; the answer checks that every
//! instance read its own.

/// `groups` x 8 leaf instances under two generate levels.
fn design(groups: usize) -> String {
    format!(
        r#"
module leaf #(parameter int N = 2, parameter int W = 4) (input logic [7:0] in, output logic o);
  typedef struct packed {{ logic a; logic [W-1:0] d; }} s_t;
  s_t [N-1:0] par;
  assign par = {{N{{in[0], in[W:1]}}}};
  assign o = $countones(par[1].d) > 1;
endmodule
module top;
  localparam int W = 7;
  localparam int G = {groups}, H = 8;
  wire [G*H-1:0] o;
  for (genvar g = 0; g < G; g++) begin : ga
    for (genvar h = 0; h < H; h++) begin : gb
      leaf #(.W(3 + h % 2)) u (.in(8'(g * H + h)), .o(o[g * H + h]));
    end
  end
  initial begin
    int want, bad, c, w;
    logic [7:0] v;
    want = 0;
    bad = -1;
    #1;
    for (int i = 0; i < G * H; i++) begin
      w = 3 + (i % H) % 2;
      v = 8'(i);
      c = 0;
      for (int b = 1; b <= w; b++) c += v[b];
      want += (c > 1);
      if (o[i] !== (c > 1) && bad < 0) bad = i;
    end
    $display("DECL_SCOPE ones=%0d want=%0d first_bad=%0d", $countones(o), want, bad);
  end
endmodule
"#
    )
}

/// The design's answer and the work counter after it settles.
fn run(groups: usize) -> (String, u64) {
    let sim = xezim::simulate(&design(groups), 10).expect("simulate the instance design");
    let line = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with("DECL_SCOPE"))
        .unwrap_or_else(|| panic!("no DECL_SCOPE line in {:?}", sim.output));
    (line, sim.decl_param_rows())
}

#[test]
fn instance_scope_parameter_work_grows_with_instances() {
    let (small_line, small) = run(8);
    let (large_line, large) = run(32);
    // 64 and 256 leaves: the ones are those whose own `in[W:1]` holds two.
    assert_eq!(small_line, "DECL_SCOPE ones=38 want=38 first_bad=-1");
    assert_eq!(large_line, "DECL_SCOPE ones=152 want=152 first_bad=-1");
    assert!(
        small > 0,
        "the selects did not size their declarations in a scope"
    );
    // Four times the instances: four times the work, not sixteen.
    assert!(
        large <= small * 6,
        "parameter rows read: {small} for 64 instances, {large} for 256"
    );
}
