class configuration; bit exclusive_access_enable=0;bit exclusive_monitor_enable=1;endclass
class transaction;
 typedef enum {OKAY=0,EXOKAY=1,SLVERR=2,DECERR=3,PREFETCHED=4,FAULT=5,UNSUPPORTED=7} response_t;
 rand response_t resp[];rand response_t bresp;bit atomic_type=0;
 constraint size_c {resp.size()==8;foreach(resp[i]) resp[i] inside {OKAY,SLVERR,DECERR,PREFETCHED};}
endclass
class sequence_c;
 transaction req=new();configuration cfg=new();
 int OKAY_wt=100, EXOKAY_wt=10, SLVERR_wt=0, DECERR_wt=1;
 task response();
 if(!req.randomize with {
   if(!(cfg.exclusive_access_enable == 1 && cfg.exclusive_monitor_enable == 1 && atomic_type == 1)) {
 bresp dist {transaction::OKAY:=OKAY_wt,transaction::EXOKAY:=EXOKAY_wt,transaction::SLVERR:=SLVERR_wt,transaction::DECERR:=DECERR_wt};
 foreach(this.resp[index]) resp[index] dist {
     transaction::OKAY:=OKAY_wt,transaction::EXOKAY:=EXOKAY_wt,
     transaction::SLVERR:=SLVERR_wt,transaction::DECERR:=DECERR_wt};
 }
 foreach(this.resp[index]) this.resp[index] != transaction::DECERR;
 })$fatal(1,"randomize failed");
 foreach(req.resp[i]) if(req.resp[i]!=transaction::OKAY)$fatal(1,"invalid resp %0d",req.resp[i]);
 endtask
endclass
module top;
 initial begin sequence_c s=new();repeat(10)s.response();$display("PASS");$finish;end
endmodule
