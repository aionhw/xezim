// A class declared in the compilation-unit scope ($unit), with out-of-block
// methods (IEEE 1800-2023 8.24), and a top module that declares variables
// with the same names as members of the class.
//
//   xezim --simulate -s top --no-cache repro.sv
//
// Every line should print PASS. With the bug, elaboration stops with
//   Undefined variable 'arr': referenced before its declaration in module 'top'
// and nothing runs.
//
// An out-of-block method body is in the scope of its class: 8.24 says it
// "shall be able to access all declarations of the class", and 23.9 searches
// the enclosing scopes of a name from the inside out. The method is not part
// of module top, so the module's variables are not candidates for these names.

class base_c;
  int b;                                // member of the base class
endclass

class c extends base_c;
  int v;
  int arr [3];
  extern function void set(int x);      // writes members by their bare names
  extern function int  get();           // reads a member by its bare name
  extern task          bump();          // compound assignment and increment
  extern function void set_base(int x); // writes a member of the base class
endclass

function void c::set(int x);
  v = x;
  arr[1] = x + 2;
endfunction

function int c::get();
  return v;
endfunction

task c::bump();
  v += 5;
  v++;
endtask

function void c::set_base(int x);
  b = x;
endfunction

module top;
  // The same names as the class members. They are different objects.
  int v = 100;
  int arr [3];
  int b = 300;
  int fails = 0;

  task automatic check(string name, int got, int want);
    if (got == want) $display("PASS %s", name);
    else begin
      $display("FAIL %s: got %0d, expected %0d", name, got, want);
      fails++;
    end
  endtask

  initial begin
    c o;
    o = new;
    o.set(7);
    check("function_writes_member", o.v, 7);
    check("function_writes_array_member", o.arr[1], 9);
    check("module_variable_untouched", v, 100);
    check("module_array_untouched", arr[1], 0);
    check("function_reads_member", o.get(), 7);
    o.bump();
    check("task_compound_and_increment", o.v, 13);
    o.set_base(5);
    check("base_member_written", o.b, 5);
    check("module_base_name_untouched", b, 300);
    if (fails == 0) $display("ALL PASS");
    else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
