`timescale 1ps/1ps
module fine;
endmodule

`timescale 1ns/100ps
module child(input bit enable, clk);
  timeunit 1us;
  timeprecision 100ns;
  real from_initial, from_comb, from_edge;
  int initial_rc, comb_rc, edge_rc;

  initial begin
    #1;
    initial_rc = $sscanf("10.345", "%t", from_initial);
  end

  // §9.2.2.2: gate the scan until top has set the shared $timeformat.
  always_comb begin
    if (enable) begin
      comb_rc = $sscanf("10.345", "%t", from_comb);
    end else begin
      comb_rc = 0;
      from_comb = 0.0;
    end
  end

  // §9.4.2: top samples after the clock edge and the combinational update.
  always @(posedge clk) edge_rc = $sscanf("10.345", "%t", from_edge);

  // §13.4: a hierarchical call executes in the declaring module's time unit.
  function automatic real scan();
    real value;
    int rc;
    rc = $sscanf("10.345", "%t", value);
    return rc == 1 ? value : -1.0;
  endfunction
endmodule

module top;
  bit enable, clk;
  real t;
  int rc;
  fine precision_source();
  child u(enable, clk);

  initial begin
    // Tables 20-3 and 21-7: round 1250.5 to 1251 global precision ticks.
    rc = $sscanf("1250.5", "%t", t);
    if (rc != 1 || t != 1.251) $fatal(1, "default $timeformat: rc=%0d t=%f", rc, t);
    $timeformat(-3, 2, " ms", 10);
    if (u.scan() != 10350.0) $fatal(1, "hierarchical function");
    #10 enable = 1;
    #10 clk = 1;
    #1100;
    if (u.initial_rc != 1 || u.from_initial != 10350.0) $fatal(1, "child initial");
    if (u.comb_rc != 1 || u.from_comb != 10350.0) $fatal(1, "child always_comb");
    if (u.edge_rc != 1 || u.from_edge != 10350.0) $fatal(1, "child posedge");
    $display("PASS");
  end
endmodule
