// IEEE 1800-2023 §13.5.1, §11.8.2: implicit integral formals use
// their declared packed dimensions, just as explicit logic formals do.
package implicit_formal_pkg;
  class mapper #(int W = 4);
    static function int bits_of(input [W-1:0] v);
      return $bits(v);
    endfunction
    static function int high_of(input [W-1:0] v);
      return $high(v);
    endfunction
    static function [W-1:0] mix(input [W-1:0] v);
      return v ^ v[W-1:1];
    endfunction
    static function [W-1:0] explicit_mix(input logic [W-1:0] v);
      return v ^ v[W-1:1];
    endfunction
    static function [W-1:0] default_mix(input [W-1:0] v = 5);
      return v ^ v[W-1:1];
    endfunction
    static function [W-1:0] wrap(input [W-1:0] v);
      return mix(v);
    endfunction
    static function [W-1:0] eachbit(input [W-1:0] v);
      logic [W-1:0] result;
      result = '0;
      foreach (v[k]) result[k] = v[k] ^ v[W-1];
      return result;
    endfunction
    static function int signed_value(input signed [W-1:0] v);
      return v;
    endfunction
    static function [W-1:0] add(input [W-1:0] v);
      return v;
    endfunction
    static function void copy_out(input [W-1:0] v, output [W-1:0] o);
      o = v ^ v[W-1:1];
    endfunction
    static task copy_inout(inout [W-1:0] v);
      v = v ^ v[W-1:1];
    endtask
    function [W-1:0] instance_mix(input [W-1:0] v);
      return v ^ v[W-1:1];
    endfunction
  endclass
endpackage

class implicit_literal;
  int seen;
  function new(input [3:0] v);
    seen = v ^ v[3:1];
  endfunction
  function int bits_of(input [3:0] v);
    return $bits(v);
  endfunction
  function int arithmetic_bounds(input [4-1:0] v);
    return $bits(v);
  endfunction
  function [3:0] arithmetic_mix(input [4-1:0] v);
    return v ^ v[3:1];
  endfunction
  task mix(input [3:0] v, output [3:0] o);
    o = v ^ v[3:1];
  endtask
endclass

module top;
  import implicit_formal_pkg::*;
  bit [2:0] narrow = 5;
  bit [7:0] a = 200, b = 100;
  logic [3:0] result, io;
  mapper #(4) inst;
  implicit_literal literal_inst;
  int failures = 0;
  task automatic check(input string name, input logic ok);
    if (ok !== 1'b1) begin
      $display("FAIL %s", name);
      failures++;
    end
  endtask
  initial begin
    inst = new;
    literal_inst = new(narrow);
    check("constructor", literal_inst.seen === 7);
    check("literal width", literal_inst.bits_of(narrow) === 4);
    check("arithmetic bounds", literal_inst.arithmetic_bounds(narrow) === 4);
    check("arithmetic bound value", literal_inst.arithmetic_mix(narrow) === 4'd7);
    check("parameter width", mapper#(4)::bits_of(narrow) === 4);
    check("parameter high", mapper#(4)::high_of(narrow) === 3);
    check("parameter value", mapper#(4)::mix(narrow) === 4'd7);
    check("explicit control", mapper#(4)::explicit_mix(narrow) === 4'd7);
    check("default", mapper#(4)::default_mix() === 4'd7);
    check("nested call", mapper#(4)::wrap(narrow) === 4'd7);
    check("foreach formal", mapper#(4)::eachbit(narrow) === 4'd5);
    check("instance", inst.instance_mix(narrow) === 4'd7);
    check("signed source", mapper#(4)::signed_value(3'sb101) === -3);
    check("unsigned source", mapper#(4)::signed_value(3'b101) === 5);
    check("context arithmetic", mapper#(9)::add(a + b) === 9'd300);
    check("real conversion", mapper#(4)::add(2.6) === 4'd3);
    check("truncation", mapper#(4)::add(8'hf5) === 4'd5);
    mapper#(4)::copy_out(narrow, result);
    check("output", result === 4'd7);
    io = 5;
    mapper#(4)::copy_inout(io);
    check("inout", io === 4'd7);
    literal_inst.mix(narrow, result);
    check("instance task", result === 4'd7);
    for (int i = 0; i < 1200; i++) begin
      check("hot width", mapper#(4)::bits_of(narrow) === 4);
      check("hot static", mapper#(4)::mix(narrow) === 4'd7);
      check("hot instance", inst.instance_mix(narrow) === 4'd7);
      check("hot foreach", mapper#(4)::eachbit(narrow) === 4'd5);
      check("hot arithmetic bounds", literal_inst.arithmetic_mix(narrow) === 4'd7);
    end
    if (failures) $fatal(1, "FAIL count=%0d", failures);
    $display("PASS implicit formal widths");
    $finish;
  end
endmodule
