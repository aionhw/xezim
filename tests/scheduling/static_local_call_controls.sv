module top;
  logic clk=0; always #5 clk=~clk;
  int calls=0, shared_result=0, nested_result=0, branch_result=0;
  int written_sum=0, automatic_result=0;
  int x=0, y=0;
  task automatic sample(input int value, output int result);
    static int k;
    k=value;
    #2 result=k;
  endtask
  initial sample(1,x);
  initial begin #1; sample(2,y); end
  function automatic void shared(input bit advance);
    static int k;
    if (advance) k++;
    shared_result=k;
  endfunction
  task automatic nested();
    if (1) begin : inner
      static int k;
      k+=2; nested_result=k;
    end
  endtask
  function automatic void branched(input bit reset);
    static int k;
    if (reset) k=7; else k+=3;
    branch_result=k;
  endfunction
  function automatic void written(input int x);
    static int k;
    k=x*2; written_sum+=k;
  endfunction
  function void automatic_local();
    automatic int k;
    k++; automatic_result=k;
  endfunction
  always @(posedge clk) begin
    calls++;
    shared(1); shared(0); nested(); branched(calls==1);
    written(calls); automatic_local();
  end
  initial begin
    repeat (4) @(posedge clk);
    #1 $display("T|shared=%0d nested=%0d branch=%0d written=%0d automatic=%0d x=%0d y=%0d",shared_result,nested_result,branch_result,written_sum,automatic_result,x,y);
    $finish;
  end
endmodule
