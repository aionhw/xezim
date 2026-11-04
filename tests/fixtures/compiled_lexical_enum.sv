class base;
 typedef enum bit[2:0] {READ=0,WRITE=1,IDLE=2,SHARED_OP=3,STREAM_OP=4} kind_t;
 typedef enum {RD_PLAIN=0,RD_ONCE=1,WR_PLAIN=14,WR_BACK=18} coh_t;
 kind_t kind=SHARED_OP;
 coh_t coh=WR_PLAIN;
 extern function kind_t channel();
endclass
function base::kind_t base::channel();
 begin
 if(kind==READ) channel=READ;
 else if(kind==WRITE) channel=WRITE;
 else if(kind==STREAM_OP) channel=STREAM_OP;
 else if(kind==SHARED_OP) begin
 case(coh)
 RD_PLAIN,RD_ONCE:channel=READ;
 WR_PLAIN,WR_BACK:channel=WRITE;
 endcase
 end
 end
endfunction
class other; typedef enum {RD_PLAIN=14,OTHER=2} bad; endclass
class child extends base;endclass
module top;
 initial begin child t=new();
 for(int i=0;i<2005;i++) if(t.channel()!=base::WRITE) $fatal(1,"iteration=%0d got=%0d",i,t.channel());
 $display("PASS");$finish;
 end
endmodule
