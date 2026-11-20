// IEEE 1800-2023 §5.7.1, §6.12.1, §10.7: an unbased unsized literal stored into a
// real, shortreal or realtime target converts at its self-determined one bit
// (`r <= '1` stores 1.0), in compiled and interpreted assignments, continuous
// assignments, collection stores and port connections; it was widened to the
// target's 64-bit storage first (2**64-1, or NaN for a delayed update).
module sub(input real p, output real got);
  assign got = p;
endmodule
module top;
  real a_blk, a_nba, a_dly, i_nba, i_dly, i_blk, c_var, rq[$], ra[string], rs[2], rp, rp0, a_arr[2], a_cast;
  shortreal sa_blk, sa_nba;
  realtime rt_nba;
  bit go = 0;
  assign c_var = '1;
  sub u(.p('1), .got(rp));
  sub u0(.p('0), .got(rp0));
  always @(posedge go) begin
    a_blk = '1;
    a_nba <= '1;
    sa_blk = '1;
    sa_nba <= '1;
    rt_nba <= '1;
    a_arr[1] <= '1;
    a_cast = real'('1);
  end
  always @(posedge go) a_dly <= #1 '1;
  initial begin
    i_blk = '1;
    i_nba <= '1;
    i_dly <= #1 '1;
    rq.push_back('1);
    rq.push_front('0);
    ra["k"] = '1;
    rs[1] = '1;
    rs[0] = 'x;
    #1 go = 1;
    #3;
    $display("T|alw blk %g nba %g dly %g sblk %g snba %g rt %g arr %g cast %g", a_blk, a_nba, a_dly, sa_blk, sa_nba, rt_nba, a_arr[1], a_cast);
    $display("T|ini blk %g nba %g dly %g cont %g", i_blk, i_nba, i_dly, c_var);
    $display("T|store q %g %g a %g s %g %g port %g %g", rq[1], rq[0], ra["k"], rs[1], rs[0], rp, rp0);
    $finish;
  end
endmodule
