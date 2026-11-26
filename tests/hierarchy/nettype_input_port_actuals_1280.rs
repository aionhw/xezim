//! IEEE 1800-2023 §23.3.3: "If only one of the two connections is of a
//! user-defined nettype then ... the connection shall be treated as a
//! continuous assignment from source to sink." An input port of a
//! user-defined nettype connected to a variable, a literal, a member or an
//! expression keeps its own net, which the nettype's resolution function
//! resolves (§6.6.7); only a net of the same nettype merges with it (#1280).
//! Expected values are the reference simulator's.

fn t_lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100).expect("simulate nettype port actuals");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("T|"))
        .collect()
}

#[test]
fn nettype_input_port_resolves_a_real_variable_actual() {
    let lines = t_lines(
        r#"`timescale 1ns/1ps
function automatic real res(input real d[]);
  res = 10.0;
  foreach (d[i]) res += d[i];
endfunction
nettype real N with res;
module sink(input N x, output real y);
  assign y = x;
endmodule
module top;
  real drive = 1.25, got;
  N control;
  sink s(drive, got);
  assign control = drive;
  initial begin
    #1 $display("T|drive=%g got=%g control=%g", drive, got, control);
  end
endmodule
"#,
    );
    assert_eq!(lines, ["T|drive=1.25 got=11.25 control=11.25"]);
}

#[test]
fn nettype_input_port_actual_shapes() {
    let lines = t_lines(include_str!("nettype_input_port_actuals_1280.sv"));
    assert_eq!(
        lines,
        [
            "T|var=11.25 lit=11.25 pkg=101.25 expr=11.25 net=11.25 struct=11.25 out=12 named=11.25 inner=14.25",
            "T|var=12.5 lit=11.25 pkg=102.5 expr=12.5 net=12.5 named=12.5 inner=15.5 drive=2.5",
        ]
    );
}
