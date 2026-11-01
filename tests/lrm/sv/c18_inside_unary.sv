// `inside` / `dist` items built with a unary operator of a non-rand value.
// `~k` is context-determined (sized with the operand and the other items),
// while `!k` and the reductions are 1-bit. An item the solver cannot read
// must not drop out of the set.
class c18iu_item;
  rand bit        en;
  rand bit  [3:0] n;
  rand bit  [7:0] m;
  rand int        x;
endclass

module c18iu;
  initial begin
    c18iu_item r = new();
    bit cur = 0;
    bit [3:0] k = 4'h5;
    int ok, toggles, seen5, seena;

    ok = r.randomize() with { n inside {~k}; };
    $display("T|inside_unary|bitnot ok=%0d n=%h", ok, r.n);
    ok = r.randomize() with { m inside {~k}; };
    $display("T|inside_unary|bitnot widened ok=%0d m=%h", ok, r.m);
    ok = r.randomize() with { x inside {~k}; };
    $display("T|inside_unary|bitnot int ok=%0d x=%0d", ok, r.x);
    ok = r.randomize() with { en inside {!cur}; };
    $display("T|inside_unary|lognot ok=%0d en=%b", ok, r.en);
    ok = r.randomize() with { en inside {^k}; };
    $display("T|inside_unary|xor-reduce ok=%0d en=%b", ok, r.en);
    ok = r.randomize() with { en dist {|k := 1}; };
    $display("T|inside_unary|or-reduce dist ok=%0d en=%b", ok, r.en);

    seen5 = 0;
    seena = 0;
    repeat (100) begin
      void'(r.randomize() with { n inside {k, ~k}; });
      if (r.n == 4'h5) seen5++;
      if (r.n == 4'ha) seena++;
    end
    $display("T|inside_unary|pair only=%0d both=%0d", seen5 + seena == 100, seen5 > 0 && seena > 0);

    toggles = 0;
    repeat (300) begin
      void'(r.randomize() with { en dist { cur :/ 2, ~cur :/ 1 }; });
      if (r.en != cur) toggles++;
      cur = r.en;
    end
    $display("T|inside_unary|dist toggles some=%0d", toggles > 30 && toggles < 270);
    $finish;
  end
endmodule
