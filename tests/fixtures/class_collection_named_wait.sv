class counter_base;
 int count;
 function int num(input int ignored=0);return count;endfunction
 function int size(input int ignored=0);return count;endfunction
endclass
class counter extends counter_base;endclass
class owner;
 counter q=new();
 int actual_queue[$];
 bit num_done,size_done,queue_done;
 task wait_num();wait(q.num(q.count)!=0);num_done=1;endtask
 task wait_size();wait(q.size(q.count)!=0);size_done=1;endtask
 task wait_queue();wait(actual_queue.size()!=0);queue_done=1;endtask
endclass
module top;
 bit clk;
 always #1 clk=~clk;
 initial begin
 owner s=new();
 fork s.wait_num();s.wait_size();s.wait_queue(); join_none
 #5;s.q.count=1;s.actual_queue.push_back(42);
 #1;if(!s.num_done || !s.size_done || !s.queue_done)$fatal(1,"missed wait");
 $display("PASS");$finish;
 end
endmodule
