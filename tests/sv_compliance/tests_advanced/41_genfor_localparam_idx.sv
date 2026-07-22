// §27.4: localparam declared inside a generate-for body can reference the
// genvar. The elaborate phase must substitute the genvar into the initializer
// so each iteration gets its per-iteration constant.
//
// Mirrors rr_arb_tree: nested generate-for loops create a binary-tree of
// localparams `Idx0 = 2**level-1+l` that index a continuous-assign LHS.
// If genvar substitution is missing, every Idx0 resolves to 0 and vec[0] is
// driven multiple times while higher bits stay X.
// Expected output: TEST_PASS
module inner #(parameter int unsigned NumIn = 5) ();
  if (NumIn == unsigned'(1)) begin : gen_pass
    logic dummy;
  end else begin : gen_arbiter
    localparam int unsigned NumLevels = unsigned'($clog2(NumIn));
    logic [2**NumLevels-2:0] vec;

    for (genvar level = 0; unsigned'(level) < NumLevels; level++) begin : gen_levels
      for (genvar l = 0; l < 2**level; l++) begin : gen_level
        localparam int unsigned Idx0 = 2**level-1+l;
        assign vec[Idx0] = 1'b1;
      end
    end

    initial begin
      #1;
      if (vec === 7'b1111111) $display("TEST_PASS genfor localparam idx vec=%b", vec);
      else                    $display("TEST_FAIL genfor localparam idx vec=%b (expected 1111111)", vec);
    end
  end
endmodule

module top;
  inner #(.NumIn(5)) u_inner ();
endmodule
