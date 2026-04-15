`include "../common/svtest_defs.svh"

module test_control_flow;
  `SVTEST_INIT

  int sum;
  int i;
  int case_out;
  int count_down;
  int dw;

  initial begin
    sum = 0;
    for (i = 0; i < 4; i++) begin
      sum += i;
    end
    `SVTEST_CHECK(sum == 6, "for loop failed")

    case (3)
      1: case_out = 10;
      2: case_out = 20;
      3: case_out = 30;
      default: case_out = -1;
    endcase
    `SVTEST_CHECK(case_out == 30, "case statement failed")

    count_down = 3;
    while (count_down > 0) begin
      count_down--;
    end
    `SVTEST_CHECK(count_down == 0, "while loop failed")

    dw = 0;
    do begin
      dw++;
    end while (dw < 2);
    `SVTEST_CHECK(dw == 2, "do-while loop failed")

    if (sum == 6) begin
      `SVTEST_CHECK(1, "if branch")
    end else begin
      `SVTEST_CHECK(0, "if/else failed")
    end

    `SVTEST_PASSFAIL
  end
endmodule
