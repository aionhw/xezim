/// §7.2.1, §11.5.1: a selected field bit updates its class array element.
#[test]
fn class_collection_member_index_write() {
    let source = r#"
module t;
  typedef struct packed { logic [2:0] op; logic [3:0] f; } cmd_t;
  class C;
    cmd_t inst [2];
    cmd_t one;
    function void set();
      inst[0] = '0; inst[1] = '0; one = '0;
      inst[0].op[1] = 1'b1;
      inst[1].f[2:1] = 2'b11;
      inst[1].op = 3'b100;
      one.op[1] = 1'b1;
    endfunction
  endclass
  cmd_t m_inst [2];
  initial begin
    C c = new;
    c.set();
    $display("class: inst[0].op=%b inst[1].f=%b inst[1].op=%b one.op=%b", c.inst[0].op, c.inst[1].f, c.inst[1].op, c.one.op);
    c.inst[0] = '0;
    c.inst[0].op[1] = 1'b1;
    $display("via handle: inst[0].op=%b", c.inst[0].op);
    m_inst[0] = '0;
    m_inst[0].op[1] = 1'b1;
    $display("module: m_inst[0].op=%b", m_inst[0].op);
  end
endmodule
"#;
    let sim = xezim::simulate(source, 100).expect("simulation failed");
    let messages: Vec<_> = sim.output.iter().map(|line| line.message.as_str()).collect();
    assert!(messages.contains(&"class: inst[0].op=010 inst[1].f=0110 inst[1].op=100 one.op=010"), "{messages:?}");
    assert!(messages.contains(&"via handle: inst[0].op=010"), "{messages:?}");
    assert!(messages.contains(&"module: m_inst[0].op=010"), "{messages:?}");
}
