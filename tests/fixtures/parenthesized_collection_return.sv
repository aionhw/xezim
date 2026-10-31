class item;
 int id;
endclass
typedef item queue_t[$];
class holder;
 queue_t items;
 function queue_t get(); return ((items)); endfunction
 function queue_t get_local();
  queue_t local_items=items;
  return ((local_items));
 endfunction
 function int get_count(output queue_t out_items);
  out_items=get();return out_items.size();
 endfunction
endclass
module top;
 initial begin
 holder h=new();item it=new();queue_t q;int n;
 q=h.get();if(q.size()!=0)$fatal(1,"empty member queue return");
 q=h.get_local();if(q.size()!=0)$fatal(1,"empty local queue return");
 repeat(1100) begin
 n=h.get_count(q);if(n!=0 || q.size()!=0)$fatal(1,"empty output queue");
 end
 it.id=157;h.items.push_back(it);
 q=h.get();if(q.size()!=1 || q[0]!=it || q[0].id!=157)$fatal(1,"member queue return");
 q=h.get_local();if(q.size()!=1 || q[0]!=it)$fatal(1,"local queue snapshot");
 h.items.delete();q=h.get();if(q.size()!=0)$fatal(1,"emptied member queue return");
 $display("PASS");$finish;
 end
endmodule
