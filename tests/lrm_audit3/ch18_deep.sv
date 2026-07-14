// Ch.18 deeper: dist edge cases, inside with arrays, implication chains, randc
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[18] %s", name); fails++; end
  class C;
    rand int a, b;
    rand bit [3:0] arr[];
    constraint c_sz { arr.size() == 4; }
    constraint c_ord { solve a before b; a inside {[1:3]}; b > a; b < 10; }
    constraint c_impl { (a == 2) -> (b == 9); }
  endclass
  class D;
    rand bit [2:0] v;
    constraint c { v dist {0 := 1, [1:2] :/ 4, 7 := 5}; }
  endclass
  initial begin
    C c = new(); D d = new();
    int ok = 1, impl_seen = 0;
    int hist[8];
    repeat (50) begin
      if (!c.randomize()) ok = 0;
      if (!(c.a inside {[1:3]})) ok = 0;
      if (!(c.b > c.a && c.b < 10)) ok = 0;
      if (c.a == 2 && c.b != 9) ok = 0;
      if (c.a == 2) impl_seen = 1;
      if (c.arr.size() != 4) ok = 0;
    end
    `CK("solve-before + implication + size", ok == 1)
    ok = 1;
    repeat (400) begin
      void'(d.randomize());
      hist[d.v]++;
      if (!(d.v inside {0, 1, 2, 7})) ok = 0;
    end
    `CK("dist stays in support", ok == 1)
    `CK("dist zero-support values never drawn", hist[3] == 0 && hist[4] == 0 && hist[5] == 0 && hist[6] == 0)
    `CK("dist all supported values seen", hist[0] > 0 && hist[7] > 0 && (hist[1] + hist[2]) > 0)
    `CK("dist weights ordered (7 heaviest)", hist[7] > hist[0])
    begin // inside with an array operand
      int pool[4];
      int x, hits;
      pool = '{2, 4, 6, 8};
      hits = 0;
      repeat (30) begin
        void'(std::randomize(x) with { x inside {pool}; });
        if (!(x inside {2, 4, 6, 8})) hits++;
      end
      `CK("inside {array}", hits == 0)
    end
    $display("CH18 CHECKS DONE fails=%0d", fails);
  end
endmodule
