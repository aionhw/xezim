class shared_state;
 logic a=0,b=0;
 logic[127:0] payload=0;
endclass
class watcher;
 shared_state s;
 int hits,comma_hits,single_hits;
 function new(shared_state state_handle);s=state_handle;endfunction
 task run();forever begin @(s.a or s.b);hits++;end endtask
 task comma_run();forever begin @(s.a,s.payload);comma_hits++;end endtask
 task single_run();forever begin @(s.b);single_hits++;end endtask
endclass
module top;
 initial begin
 shared_state s=new();watcher w=new(s);
 fork w.run();w.comma_run();w.single_run(); join_none
 #5;if(w.hits!=0 || w.comma_hits!=0 || w.single_hits!=0)$fatal(1,"event woke without change");
 s.a=1;#1;if(w.hits!=1 || w.comma_hits!=1)$fatal(1,"first change missed");
 s.b=1;#1;if(w.hits!=2 || w.comma_hits!=1 || w.single_hits!=1)$fatal(1,"second change missed");
 s.a=0;#1;if(w.hits!=3 || w.comma_hits!=2)$fatal(1,"independent falling change missed");
 s.payload=128'h80000000000000000000000000000000;
 #1;if(w.comma_hits!=3 || w.hits!=3)$fatal(1,"128-bit change missed");
 s.b=1;#1;if(w.hits!=3 || w.single_hits!=1)$fatal(1,"same value woke event");
 s.b='x;#1;if(w.hits!=4 || w.single_hits!=2)$fatal(1,"X transition missed");
 s.b=0;#1;if(w.hits!=5 || w.single_hits!=3)$fatal(1,"transition out of X missed");
 $display("PASS");$finish;
 end
endmodule
