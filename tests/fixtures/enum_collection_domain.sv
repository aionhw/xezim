class txn;
 typedef enum bit[3:0] {OKAY=0, EXOKAY=1, SLVERR=2, DECERR=3} resp_t;
 rand resp_t resp[];
 constraint c {resp.size()==8; foreach(resp[i]) resp[i]!=EXOKAY;}
endclass
module top;
 initial begin
 txn t=new();
 repeat(10) begin
 if(!t.randomize()) $fatal(1,"rand failed");
 foreach(t.resp[i]) if(!(t.resp[i] inside {0,2,3})) $fatal(1,"illegal enum %0d",t.resp[i]);
 end
 $display("PASS");$finish;
 end
endmodule
