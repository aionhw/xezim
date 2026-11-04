// IEEE 1800-2017 10.4.2: "If the variable_lvalue requires an evaluation, such as
// an index expression, class handle, or virtual interface reference, it shall be
// evaluated at the same time as the expression on the right-hand side" (and 4.9.4:
// the values in effect when the update is scheduled determine the target). So a
// nonblocking assignment to a member of a class object updates the object that was
// named when the statement executed. Every line should print PASS.
class pkt;
  bit [7:0] sc;
  bit [7:0] data [0:3];
endclass

// Two tasks, each with a formal named `tr`, running at the same time.
class writer_a;
  task run(pkt tr);
    #5;
    tr.sc <= 8'h11;
    tr.data[1][3:0] <= 4'ha;   // part-select of an element
  endtask
endclass
class writer_b;
  task run(pkt tr);
    #5;
    tr.sc <= 8'h22;
    tr.data[1][7:4] <= 4'hb;
  endtask
endclass

// The handle is reassigned right after the nonblocking assignment.
class swapper;
  task run(pkt tr, pkt other);
    #5;
    tr.sc <= 8'h33;
    tr = other;                // must not redirect the pending update
  endtask
endclass

module t;
  pkt pa = new, pb = new, pc = new, pd = new;
  writer_a wa = new; writer_b wb = new; swapper ws = new;
  int fails = 0;
  task automatic chk(string what, logic [7:0] got, logic [7:0] exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %h, expected %h", what, got, exp); fails++; end
  endtask
  initial begin
    fork wa.run(pa); wb.run(pb); ws.run(pc, pd); join_none
    #10;
    chk("writer_a: pa.sc",      pa.sc,      8'h11);
    chk("writer_a: pa.data[1]", pa.data[1], 8'h0a);
    chk("writer_b: pb.sc",      pb.sc,      8'h22);
    chk("writer_b: pb.data[1]", pb.data[1], 8'hb0);
    chk("swapper: pc.sc (named when executed)", pc.sc, 8'h33);
    chk("swapper: pd.sc (untouched)",           pd.sc, 8'h00);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
