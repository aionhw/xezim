// EXPECT: compile_fail
class neg08_c;
  rand int a;
  constraint c_bad { a == b; }
endclass

module neg08_bad_constraint_reference;
  neg08_c c;
  initial c = new();
endmodule
