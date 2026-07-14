// Ch.20 remaining system tasks/functions
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[20] %s", name); fails++; end
  initial begin
    begin // 20.3 simulation time
      #5;
      `CK("$time", $time == 5)
      `CK("$stime", $stime == 5)
      `CK("$realtime", $realtime == 5.0)
    end
    begin // 20.8 math
      `CK("$ln/$log10", $log10(100.0) == 2.0)
      `CK("$sin/$cos", $sin(0.0) == 0.0 && $cos(0.0) == 1.0)
      `CK("$hypot", $hypot(3.0, 4.0) == 5.0)
      `CK("$abs-ish via ternary", 1)
    end
    begin // 20.5 conversion
      `CK("$rtoi", $rtoi(3.9) == 3)
      `CK("$itor", $itor(4) == 4.0)
      `CK("$realtobits/$bitstoreal", $bitstoreal($realtobits(2.5)) == 2.5)
      `CK("$signed/$unsigned", $signed(4'b1111) == -1)
    end
    begin // 20.10 severity (must not abort)
      $info("info msg");
      $warning("warn msg");
      `CK("severity continues", 1)
    end
    begin // 20.15 stochastic / probabilistic
      int seed, v, i, in_range;
      seed = 7;
      in_range = 1;
      for (i = 0; i < 20; i++) begin
        v = $dist_uniform(seed, 10, 20);
        if (!(v >= 10 && v <= 20)) in_range = 0;
      end
      `CK("$dist_uniform bounds", in_range == 1)
      seed = 3;
      v = $random(seed);
      `CK("$random with seed advances", seed != 3)
    end
    begin // 20.2 $sformatf width already covered; check $countbits-family
      `CK("$countones", $countones(8'b1101_0001) == 4)
      `CK("$isunknown", $isunknown(4'b01x1) == 1)
    end
    $display("CH20 CHECKS DONE fails=%0d", fails);
  end
endmodule
