//! IEEE 1800-2023 §9.3.2, §9.4.5: a `fork ... join_none` child does not
//! start until the parent blocks or finishes, so a child that is an
//! intra-assignment delay or event control evaluates its right-hand side
//! then, reading what the parent wrote after the fork. The blocking delay
//! form (`fork z = #2 v; join_none`) was captured when the parent forked.

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn forked_intra_assignment_reads_its_operands_when_the_child_starts() {
    let out = lines(
        r#"
module top;
  int v = 5;
  int y [2];
  int z, w, u, e;
  int w0, x0;
  event go;
  initial begin
    fork z = #2 v; join_none
    fork w = v; join_none
    fork y = #2 '{v, v}; join_none
    fork u <= #2 v; join_none
    fork e = @(go) v; join_none
    v = 9;
    #1 -> go;
    #5 $display("T|z=%0d w=%0d y=%p u=%0d e=%0d", z, w, y, u, e);
  end
  initial begin
    w0 = 1;
    fork
      x0 = #2 w0;
    join
    $display("T|join x0=%0d", x0);
  end
endmodule
"#,
    );
    assert_eq!(out, ["T|join x0=1", "T|z=9 w=9 y='{9, 9} u=9 e=9"]);
}
