//! #940 review: `p.status` compiled to `Insn::LoadProcessStatus` reads a
//! null process handle as the interpreter does (the 32-bit zero of a member
//! read through a null handle), not as RUNNING. §9.7: a live process
//! blocked in a delay is WAITING (2), as the reference simulator reports.

use xezim::simulate;

fn lines(src: &str, prefix: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with(prefix))
        .collect()
}

const SRC: &str = r#"
module top;
  class W;
    process p;
    function int fa(process q); return q.status; endfunction
    function int fb(); process q; q = p; return q.status(); endfunction
    function int fc(); return this.p.status; endfunction
  endclass
  initial begin
    W w = new;
    int a, b, c;
    for (int k = 0; k < 100; k++) begin a = w.fa(null); b = w.fb(); c = w.fc(); end
    $display("P a=%0d b=%0d c=%0d", a, b, c);
    fork begin w.p = process::self(); #5; end join_none
    #1;
    for (int k = 0; k < 100; k++) begin a = w.fa(w.p); b = w.fb(); c = w.fc(); end
    $display("Q a=%0d b=%0d c=%0d", a, b, c);
  end
endmodule
"#;

#[test]
fn process_status_of_null_and_waiting_handles() {
    assert_eq!(lines(SRC, "P "), ["P a=0 b=0 c=0"]);
    assert_eq!(lines(SRC, "Q "), ["Q a=2 b=2 c=2"]);
}
