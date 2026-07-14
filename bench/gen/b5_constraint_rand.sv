// B5 constraint-rand: solver + PRNG throughput
module bench_constraint_rand;
  class Pkt;
    rand bit [7:0]  kind;
    rand bit [15:0] len;
    rand bit [7:0]  payload[];
    rand bit [3:0]  tags[4];
    constraint c_kind { kind dist {0 := 1, [1:8] :/ 6, 9 := 3}; }
    constraint c_len  { len inside {[64:512]}; len % 4 == 0; }
    constraint c_size { payload.size() == 8; }
    constraint c_elem { foreach (payload[i]) payload[i] inside {[1:200]}; }
    constraint c_uniq { unique {tags[0], tags[1], tags[2], tags[3]}; }
  endclass
  int ok = 0, fails = 0;
  initial begin
    Pkt p = new();
    repeat (20000) begin
      if (p.randomize() with { len > 128; }) ok++; else fails++;
    end
    $display("BENCH_DONE randomizations=%0d failures=%0d", ok, fails);
    $finish;
  end
endmodule
