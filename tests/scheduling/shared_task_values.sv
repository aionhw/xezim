`timescale 1ns/1ps
// §13.3/§13.5 storage and copy-out; §7.4, §7.5, §7.10, §7.8 aggregates.
module top;
  typedef struct packed {logic [3:0] a; logic signed [3:0] b;} pair_t;
  int results[2];
  task automatic values(input int id, output int result);
    real r = 1.5;
    shortreal sr = 2.5;
    string s = "hello";
    pair_t p = '{4'ha, -4'sd2};
    logic [3:0] unknown = 4'bxz01;
    int a[2]; int dyn[]; int q[$]; int assoc[int];
    a[0]=id; a[1]=id+1;
    dyn = new[2]; dyn[0]=id+2; dyn[1]=id+3;
    q.push_back(id+4); assoc[7]=id+5;
    fork #1; #2; join
    result = a[0]+a[1]+dyn[0]+dyn[1]+q[0]+assoc[7];
    if (r != 1.5 || sr != 2.5 || s != "hello" || p.a != 10 || p.b != -2 || unknown !== 4'bxz01)
      $fatal(1,"local values changed");
  endtask
  task static shared(input int id);
    static int n=0;
    n++;
    fork #1; #2; join
    if(n != 2) $fatal(1,"static storage separated");
  endtask
  initial begin
    fork values(1,results[0]); values(10,results[1]); join
    $display("T|values %0d %0d",results[0],results[1]);
    fork shared(1); shared(2); join
    $display("T|shared done");
    $finish;
  end
endmodule
