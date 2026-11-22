module tb;
  reg hot;
  reg [63:0] sel, data;
  wire [63:0] q, v, rev;
  reg [63:0] alt;
  for (genvar i=0; i<64; i=i+1) begin
    assign q[i] = sel[i] ? hot : data[i];
    assign rev[i] = sel[i] ? data[i] : hot;
    assign v[i] = sel[i] ? data[i] : alt[i];
  end
  task show;
    #1 $display("T| %b %b %b", q, rev, v);
  endtask
  initial begin
    hot=0; sel=0; data='1; alt='0; show;
    hot=1; show;
    data={16{4'b01xz}}; alt={16{4'bzx10}}; show;
    sel='1; hot=1'bz; show;
    hot=1'bx; show;
    sel={32{2'b01}}; show;
    sel={16{4'b01xz}}; hot=1; show;
    sel=0; data=0; hot=1; show;
    hot=0; sel='1; show;
    sel=0; hot=1; show;
    sel='x; data='1; show;
    sel='z; data=0; show;
    sel='1; hot=1'bz; show;
    sel='x; data='z; alt='z; show;
    $finish;
  end
endmodule
