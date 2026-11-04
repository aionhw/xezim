// IEEE 1800-2017 18.3 / 7.5.2: cfg.ranges is state, so cfg.ranges.size() is a
// constant (1) during randomize(). idx must be 0, and the slice chain on a, b,
// c has many solutions, so randomize() must succeed every time.
class config_t;
  int ranges[];
  function new(); ranges = new[1]; endfunction
endclass
class packet;
  config_t cfg = new();
  rand int unsigned idx;
  rand bit [63:0] unrelated;
  rand int unsigned a, b, c;
  constraint s { idx < cfg.ranges.size(); }
  constraint chain { a[15:0] == b[31:16]; b[15:0] == c[31:16]; c[15:0] == 16'h1234; a[31:16] == 16'h5678; }
endclass
module top;
  initial begin
    packet p = new();
    repeat (10) begin
      if (!p.randomize()) $fatal(1, "MISMATCH: randomize failed");
      if (p.idx != 0) $fatal(1, "MISMATCH: idx=%0d", p.idx);
      if (p.a[15:0] != p.b[31:16] || p.b[15:0] != p.c[31:16] || p.c[15:0] != 16'h1234 || p.a[31:16] != 16'h5678) $fatal(1, "MISMATCH: chain a=%h b=%h c=%h", p.a, p.b, p.c);
    end
    $display("STATE_ARRAY_SIZE_BOUND_PASS");
    $finish;
  end
endmodule
