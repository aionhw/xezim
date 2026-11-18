//! §8.25 / §13.5.1: a formal declared with a class typedef sized by a class
//! parameter (`typedef logic [AW-1:0] addr_t`) binds at that width on the
//! direct call between compiled methods, as on the first calls.

use xezim::simulate;

#[test]
fn param_typedef_formal_keeps_width_on_direct_call() {
    const SRC: &str = r#"
class mem_c #(int AW = 32);
  typedef logic [AW-1:0] addr_t;
  logic [7:0] m[addr_t];
  function bit has(addr_t a);
    return m.exists(a);
  endfunction
endclass
class user_c;
  mem_c mm;
  function bit probe(bit [31:0] a);
    return mm.has(a);
  endfunction
endclass
module top;
  initial begin
    user_c u = new();
    int bad = 0;
    u.mm = new();
    u.mm.m[32'h8000_0080] = 8'h65;
    for (int n = 0; n < 2000; n++) if (!u.probe(32'h8000_0080)) bad++;
    $display("BAD=%0d", bad);
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let line = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with("BAD="))
        .expect("no BAD line");
    assert_eq!(line, "BAD=0");
}
