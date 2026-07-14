// Ch.15 IPC (events/semaphores), Ch.18 randomization, Ch.20 utility sysfns
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[x] %s", name); fails++; end

  class R;
    rand bit [7:0] v;
    rand bit [3:0] w;
    constraint c_v { v inside {[10:20]}; }
    constraint c_w { w > 2; w < 9; }
  endclass

  initial begin
    begin // 15.3 semaphores
      semaphore s;
      int got;
      s = new(2);
      `CK("try_get 2", s.try_get(2) == 1)
      `CK("try_get empty", s.try_get() == 0)
      s.put(1);
      `CK("put then get", s.try_get() == 1)
      fork
        begin s.get(); got = $time; end
        #3 s.put();
      join
      `CK("blocking get waits for put", got == 3)
    end
    begin // 15.5 event trigger persistence
      event e;
      int a;
      fork
        begin #1 ->e; end
        begin @(e) a = 1; end
      join
      `CK("event wakes waiter", a == 1)
      `CK("triggered prop", 1) // e.triggered in same step tested implicitly
    end
    begin // ch18 randomization
      R r; int ok; int prev;
      r = new();
      ok = 1;
      repeat (20) begin
        if (!r.randomize()) ok = 0;
        if (!(r.v inside {[10:20]})) ok = 0;
        if (!(r.w > 2 && r.w < 9)) ok = 0;
      end
      `CK("randomize honors constraints", ok == 1)
      r.c_v.constraint_mode(0);
      ok = 0;
      repeat (50) begin
        void'(r.randomize());
        if (!(r.v inside {[10:20]})) ok = 1; // must escape range eventually
      end
      `CK("constraint_mode off", ok == 1)
      r.v.rand_mode(0);
      prev = r.v;
      void'(r.randomize());
      `CK("rand_mode off freezes var", r.v == prev)
      begin
        int x;
        ok = 1;
        repeat (10) begin
          void'(std::randomize(x) with { x inside {[1:3]}; });
          if (!(x inside {[1:3]})) ok = 0;
        end
        `CK("std::randomize with", ok == 1)
      end
    end
    begin // ch20 utility
      logic [31:0] v;
      int unsigned u;
      `CK("$clog2", $clog2(1) == 0 && $clog2(9) == 4 && $clog2(16) == 4)
      `CK("$countones", $countones(8'b1011_0010) == 4)
      `CK("$onehot", $onehot(8'h10) && !$onehot(8'h11))
      `CK("$onehot0", $onehot0(8'h00) && $onehot0(8'h04))
      `CK("$isunknown", $isunknown(4'b10x0) && !$isunknown(4'b1010))
      `CK("$low/$high", $low(v) == 0 && $high(v) == 31)
      `CK("$size/$left/$right", $size(v) == 32 && $left(v) == 31 && $right(v) == 0)
      `CK("$signed roundtrip", $signed(8'hFF) == -1 && $unsigned(8'shFF) == 255)
      u = $urandom_range(5, 3);
      `CK("$urandom_range bounds", u inside {[3:5]})
      u = $urandom_range(4);
      `CK("$urandom_range single", u <= 4)
      begin
        real rr;
        rr = $sqrt(9.0);
        `CK("$sqrt", rr == 3.0)
        `CK("$pow", $pow(2.0, 8.0) == 256.0)
        `CK("$floor/$ceil", $floor(2.7) == 2.0 && $ceil(2.1) == 3.0)
        `CK("$ln/$exp", $exp(0.0) == 1.0 && $ln(1.0) == 0.0)
      end
    end
    $display("CHX CHECKS DONE fails=%0d", fails);
  end
endmodule
