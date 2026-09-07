//! Issue #155: a class declared inside a module reaches a SIBLING module
//! instance by hierarchical reference from its methods (the UVM-MS proxy
//! pattern). Reads returned 0 and calls were dropped, silently. Four defects
//! lined up: the object's birth scope was installed as the resolution hint
//! in `%m` form (`tb.u_w`) which no signal key matches; the resolver
//! applied the hint only to single-segment names and `a.b` parses as a
//! member access anyway; a method called through a hierarchical handle
//! (`u_w.p.peek()`) parses as one identifier path, matched no subroutine and
//! fell through to 0; and a module-scope class recorded no declaring module,
//! so an object built elsewhere had no scope at all. Reference-verified.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn expect(msgs: &[String], wants: &[&str]) {
    for want in wants {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
}

#[test]
fn proxy_reads_and_calls_sibling_instance() {
    let msgs = messages(
        "module core_m;
  int seq = 7;
  function automatic int get_seq(); get_seq = seq; endfunction
  function automatic int set_drive(input int v); seq = seq + v; set_drive = 1; endfunction
endmodule
module wrapper;
  int wvar = 3;
  class proxy_c;
    function int peek();     peek = core.get_seq(); endfunction
    function int peek_var(); peek_var = core.seq; endfunction
    function int bare();     bare = wvar; endfunction
    function void push(input int v); void'(core.set_drive(v)); endfunction
  endclass
  core_m core();
  proxy_c p = new();
  proxy_c p_blk;
  initial p_blk = new();
endmodule
module tb;
  wrapper u_w();
  wrapper::proxy_c p_tb;
  int v;
  initial begin
    #1 p_tb = new();
    $display(\"R1 %0d %0d %0d\", u_w.p.peek(), u_w.p.peek_var(), u_w.p.bare());
    $display(\"R2 %0d %0d %0d\", u_w.p_blk.peek(), u_w.p_blk.peek_var(), u_w.p_blk.bare());
    $display(\"R3 %0d %0d %0d\", p_tb.peek(), p_tb.peek_var(), p_tb.bare());
    u_w.p.push(5);
    v = u_w.p.peek_var();
    $display(\"W1 %0d %0d\", u_w.core.seq, v);
    $finish;
  end
endmodule
",
    );
    expect(&msgs, &["R1 7 7 3", "R2 7 7 3", "R3 7 7 3", "W1 12 12"]);
}
