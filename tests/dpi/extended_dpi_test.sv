module extended_dpi_test;
  import "DPI-C" function longint add64(input longint a, input longint b);
  import "DPI-C" function real scale(input real a, input real b);
  import "DPI-C" function void bump(inout int x);
  import "DPI-C" function void split(input int a, input int b, output int s);
  import "DPI-C" function chandle make_handle(input int x);
  import "DPI-C" function int use_handle(input chandle h);
  import "DPI-C" function string greet(input string s);

  int x;
  int s;
  longint l;
  real r;
  chandle h;
  int hv;
  string gs;

  initial begin
    x = 10;
    bump(x);
    split(3, 4, s);
    l = add64(64'd5, 64'd6000000000);
    r = scale(1.5, 2.0);
    h = make_handle(7);
    hv = use_handle(h);
    gs = greet("dpi");

    $display("DPI_EXT x=%0d s=%0d l=%0d r=%0f hv=%0d gs=%s", x, s, l, r, hv, gs);
    if ((x == 11) && (s == 7) && (l == 64'd6000000005) && (hv == 21) && (gs == "hello_dpi")) begin
      $display("TEST_PASS");
    end else begin
      $display("TEST_FAIL");
    end
    $finish;
  end
endmodule
