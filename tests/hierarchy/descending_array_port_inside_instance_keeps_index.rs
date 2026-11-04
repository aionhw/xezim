//! IEEE 1800-2017 §7.6 / §23.3.3: a descending unpacked array declared INSIDE
//! an instantiated module keeps its declared index order when connected to a
//! port. The instance variable path recorded only the normalized `(lo, hi)`
//! shape — never `unpacked_decl_dims` / `descending_arrays` — so once ports
//! were paired from declared bounds, `w[15:0]` was walked as `[0:15]`:
//! element 0 of the child's port received the actual's highest index, and
//! the output port reversed it back, hiding the swap from the parent.
//! Companion to xezim-core #89 (issue #88).

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn descending_array_port_inside_instance_keeps_index() {
    let sim = simulate(
        r#"
module child (
  input  logic [15:0] d [15:0],
  output logic [15:0] q [15:0]
);
  always_comb for (int i = 0; i < 16; i++) q[i] = d[i];
  initial #1 $display("child i0=%h i15=%h", d[0], d[15]);
endmodule
module mid;
  logic [15:0] w [15:0];
  logic [15:0] r [15:0];
  initial for (int i = 0; i < 16; i++) w[i] = 16'h1000 + i;
  child c (.d(w), .q(r));
  initial #2 $display("mid w0=%h w15=%h r0=%h r15=%h", w[0], w[15], r[0], r[15]);
endmodule
module wrap;
  mid m ();
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(msgs.iter().any(|m| m == "child i0=1000 i15=100f"), "{msgs:?}");
    assert!(
        msgs.iter().any(|m| m == "mid w0=1000 w15=100f r0=1000 r15=100f"),
        "{msgs:?}"
    );
}
