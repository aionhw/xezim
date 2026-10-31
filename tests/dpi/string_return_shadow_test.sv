// §26.3 / §35.5.5: formatting and execution agree about a call's owner.
package display_pkg;
  import "DPI-C" function string srd_msg(int i);
endpackage
class display_local;
  function int srd_msg(int i); return 42+i; endfunction
  function string text(); return $sformatf(srd_msg(2)); endfunction
endclass
module top;
  function automatic int srd_msg(int i); return 42+i; endfunction
  initial begin
    display_local c=new();
    string local_text=$sformatf(srd_msg(0));
    string package_text=$sformatf(display_pkg::srd_msg(1));
    if(local_text!=$sformatf("%d",42))$fatal(1,"local function formatted incorrectly");
    if(c.text()!=$sformatf("%d",44))$fatal(1,"class method formatted incorrectly");
    if(package_text!="second message")$fatal(1,"package import lost its string type");
    $display("TEST_PASS");
    $finish;
  end
endmodule
