`include "../common/svtest_defs.svh"

module test_tasks_functions;
  `SVTEST_INIT

  function automatic int add3(input int a, input int b, input int c);
    add3 = a + b + c;
  endfunction

  task automatic swap(ref int a, ref int b);
    int tmp;
    tmp = a;
    a = b;
    b = tmp;
  endtask

  task automatic make_sum(input int a, input int b, output int s);
    s = a + b;
  endtask

  int x, y, s;

  initial begin
    x = 4;
    y = 9;
    s = add3(1, 2, 3);
    `SVTEST_CHECK(s == 6, "automatic function failed")

    swap(x, y);
    `SVTEST_CHECK(x == 9 && y == 4, "task ref arguments failed")

    make_sum(7, 8, s);
    `SVTEST_CHECK(s == 15, "task output argument failed")

    `SVTEST_PASSFAIL
  end
endmodule
