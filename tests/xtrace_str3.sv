module top;
    string s;
    logic [7:0] settled;
    initial begin
        settled = 8'h42;
        s = "hi \"there\"\ttab";
        #5 s = "plain";
        #5 $finish;
    end
endmodule
