//! §11.4.1 / §10.4: assignment selectors run once before aggregate probes.
#[test]
fn part_select_rhs_preserves_single_index_evaluation() {
    let sim = xezim::simulate(r#"typedef struct packed { logic [63:0] data; } chunk;
class Packet;
  chunk slots[];
  function new(); slots=new[8]; endfunction
endclass
class Writer;
  Packet p;
  int calls;
  function new(); p=new; endfunction
  function int next_slot(); return calls++; endfunction
  task run();
    int i=0;
    logic [127:0] src=128'h1234_5678;
    #1;
    p.slots[i++].data=src[63:0];
    if(i!=1 || p.slots[0].data!=64'h12345678) $fatal(1,"index repeated");
    p.slots[next_slot()].data=src[0+:64];
    if(calls!=1 || p.slots[0].data!=64'h12345678) $fatal(1,"call repeated");
    p.slots[i++].data+=src[63:0];
    if(i!=2) $fatal(1,"compound index repeated");
    p.slots[i++].data<=src[63:0];
    #1;
    if(i!=3 || p.slots[2].data!=64'h12345678) $fatal(1,"NBA index repeated");
    $display("T|PASS");
  endtask
endclass
module top;
  initial begin Writer w=new; w.run(); $finish; end
endmodule
"#, 1000).expect("simulate");
    assert!(sim.output.iter().any(|line| line.message == "T|PASS"), "{:?}", sim.output);
}
