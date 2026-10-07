package model_pkg;
  // IEEE 1800-2023 §6.18 and §8.23: a typedef preserves the class
  // type and access to its member typedefs and static methods.
  class registry #(type T = int);
    static int calls;
    static function T create(string name = "");
      T obj; calls++; obj = new(name); return obj;
    endfunction
    static function int count(); return calls; endfunction
  endclass
  class model;
    typedef registry#(model) type_id;
    int mark;
    function new(string name = ""); mark = 7; endfunction
  endclass
  typedef model model_t;
endpackage
package env_pkg;
  typedef model_pkg::model_t rm_t;
endpackage
module top;
  import env_pkg::*;
  rm_t rm;
  initial begin
    rm = rm_t::type_id::create("rm");
    if (rm == null) $fatal(1, "factory alias returned null");
    if (rm.mark != 7) $fatal(1, "constructor did not run");
    if (rm_t::type_id::count() != 1) $fatal(1, "registry body bypassed");
    $display("FACTORY_ALIAS_PASS");
    $finish;
  end
endmodule
