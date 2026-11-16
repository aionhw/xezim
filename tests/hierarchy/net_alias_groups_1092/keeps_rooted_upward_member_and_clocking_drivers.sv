// IEEE 1800-2023 §§10.11, 6.6.3 require each protected wor bit 1; §23.6 permits hierarchical writes; §14.16: "a driver on that net shall be created" for a clocking output.
// Hierarchical/top/$root/upward port actual, packed member and clocking output targets escape textual driver counts.
`timescale 1ns/1ps
module child(input wire i); assign $root.top.d=i; endmodule
module outcell(output wire o,input wire i);assign o=i;endmodule
module top;
  wire low=0,high=1;
  wor a,b; alias a=b; assign a=low; assign top.b=high;
  wor c,d; alias c=d; assign c=low; child u(high);
  wor e,f; alias e=f; assign e=low; outcell v(.o(top.f),.i(high));
  typedef struct packed {logic x; logic y;} S;
  wor S s,t,third; alias s=t; alias third=t; assign s.x=low; assign third={high,high};
  logic clk=0;
  wor j,k; alias j=k; assign k=high;
  clocking cb @(posedge clk); output j; endclocking
  initial begin #1 clk=1; cb.j<=0; #1;
    $display("T|protected %b %b %b %b %b",b,d,f,t.x,k); $finish;
  end
endmodule
