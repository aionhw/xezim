module leaf;
  string inst_path = $sformatf("%m");
  int counter = 5;
  function automatic string const_path();
    const_path = "sub_env";
  endfunction
  function automatic string ret_path();
    return "abc";
  endfunction
  function automatic int get_int();
    return 42;
  endfunction
  function automatic int get_counter();
    return counter;
  endfunction
  function automatic string get_ip();
    return inst_path;
  endfunction
  function automatic string get_m();
    return $sformatf("%m");
  endfunction
  function string static_fn();
    static_fn = "st";
  endfunction
  function automatic string with_arg(string s);
    return {s, "_x"};
  endfunction
  task automatic t_set(output string s);
    s = "task_out";
  endtask
endmodule
module mid;
  leaf u_h();
endmodule
class C;
  string s;
  int i;
  task run();
    s = tb.m.u_h.const_path(); $display("T|cls_const_var|%s", s);
    $display("T|cls_const_inline|%s", tb.m.u_h.const_path());
    s = tb.m.u_h.ret_path(); $display("T|cls_ret|%s", s);
    i = tb.m.u_h.get_int(); $display("T|cls_int|%0d", i);
    $display("T|cls_int_inline|%0d", tb.m.u_h.get_int());
    i = tb.m.u_h.get_counter(); $display("T|cls_counter|%0d", i);
    s = tb.m.u_h.get_ip(); $display("T|cls_ip|%s", s);
    s = tb.m.u_h.get_m(); $display("T|cls_m|%s", s);
    s = tb.m.u_h.static_fn(); $display("T|cls_static|%s", s);
    s = tb.m.u_h.with_arg("q"); $display("T|cls_arg|%s", s);
    tb.m.u_h.t_set(s); $display("T|cls_task|%s", s);
    s = tb.m.u_h.inst_path; $display("T|cls_var|%s", s);
    i = tb.m.u_h.counter; $display("T|cls_cvar|%0d", i);
    s = tb.top_fn(); $display("T|cls_topfn|%s", s);
  endtask
  static function string sget();
    return tb.m.u_h.get_ip();
  endfunction
  task automatic trun();
    #1;
    s = tb.m.u_h.get_ip(); $display("T|cls_task_ip|%s", s);
  endtask
endclass
module tb;
  mid m();
  function string top_fn();
    return $sformatf("%m");
  endfunction
  initial begin
    C c = new;
    string s;
    s = m.u_h.get_ip(); $display("T|mod_ip|%s", s);
    s = m.u_h.get_m(); $display("T|mod_m|%s", s);
    s = top_fn(); $display("T|mod_topfn|%s", s);
    c.run();
    s = C::sget(); $display("T|cls_sget|%s", s);
    c.trun();
    $finish;
  end
endmodule
