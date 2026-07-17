module leaf(input logic [7:0] din, output logic [7:0] dout);
    assign dout = din + 8'h01;
endmodule
module mid(input logic [7:0] din, output logic [7:0] dout);
    leaf u_leaf(.din(din), .dout(dout));
endmodule
module top;
    logic clk; logic [15:8] hi; logic [7:0] lo; real r; event e1;
    logic [7:0] src_bus, sink_bus; logic [3:0] xz;
    logic [7:0] allx, allz; logic [7:0] mem [0:3];
    mid u_mid(.din(src_bus), .dout(sink_bus));
    initial begin
        clk=0; hi=8'h00; lo=8'h00; r=0.0; src_bus=8'h10;
        allx=8'hxx; allz=8'hzz; xz=4'b01xz;
        mem[0]=8'h00; mem[1]=8'h00; mem[2]=8'h00; mem[3]=8'h00;
    end
    initial begin #5 clk=1; #5 clk=0; #5 clk=1; #5 clk=0; #5 clk=1; #5 clk=0; end
    initial begin
        #10 r=3.25; hi=8'ha5; src_bus=8'h20; mem[1]=8'hbe; ->e1;
        #10 r=-0.5; allx=8'h00; mem[2]=8'hef; ->e1;
        #10 allz=8'h00; src_bus=8'h30; ->e1;
        #10 $finish;
    end
endmodule
