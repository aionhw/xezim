class base_transaction;
 rand int inherited;
 constraint inherited_value {inherited==7;}
endclass
class transaction extends base_transaction;
 rand int delays[];
 rand int other;
 constraint reasonable {delays.size()==4; foreach(delays[i]) delays[i] inside {[0:20]};}
endclass
module top;
 initial begin
 transaction t=new();
 t.delays=new[4];
 t.rand_mode(0);t.constraint_mode(0);
 if(t.inherited.rand_mode()!=0 || t.other.rand_mode()!=0 || t.reasonable.constraint_mode()!=0)$fatal(1,"bulk disable query");
 t.delays.rand_mode(1);t.reasonable.constraint_mode(1);
 t.inherited.rand_mode(1);t.inherited_value.constraint_mode(1);
 if(t.inherited.rand_mode()!=1 || t.other.rand_mode()!=0 || t.reasonable.constraint_mode()!=1)$fatal(1,"individual enable query");
 if(!t.randomize() with {foreach(delays[i]) delays[i] inside {[1:10]};})$fatal(1,"randomization failed");
 if(t.inherited!=7 || t.other!=0)$fatal(1,"inherited or disabled value");
 foreach(t.delays[i])if(t.delays[i]<1 || t.delays[i]>10)$fatal(1,"invalid delay");
 t.rand_mode(1);t.constraint_mode(1);
 if(t.other.rand_mode()!=1)$fatal(1,"bulk enable query");
 t.other.rand_mode(0);
 if(t.other.rand_mode()!=0 || t.inherited.rand_mode()!=1)$fatal(1,"individual disable query");
 $display("PASS");$finish;
 end
endmodule
