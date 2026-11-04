class Node;
 int tag;
 function new(int t);tag=t;endfunction
endclass
typedef struct {Node h;int number;} msg_t;
module top;
 mailbox #(msg_t) mb = new();
 Node tmp, dummy;
 msg_t result;
 int seq;
 initial begin
  for(int i=0;i<256;i++)dummy=new(i);
  dummy=null;
  tmp=new(777);
  mb.put('{h:tmp,number:99});
  tmp=null;
  #1;
  for(int j=0;j<16;j++)seq=j+10000;
  #1;
  mb.peek(result);
  $display("T|peek|%0d|%0d",result.h.tag,result.number);
  mb.peek(result);
  $display("T|peek|%0d|%0d",result.h.tag,result.number);
  mb.get(result);
  if(result.h==null)$display("T|pattern|NULL");
  else $display("T|pattern|%0d|%0d",result.h.tag,result.number);
  $finish;
 end
endmodule
