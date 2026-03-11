module tb_adder;
    logic [3:0] a, b;
    logic       cin;
    logic [3:0] sum;
    logic       cout;

    // Instantiate the Unit Under Test (UUT)
    adder_4bit_struct uut (
        .a(a),
        .b(b),
        .cin(cin),
        .sum(sum),
        .cout(cout)
    );

    initial begin
        $display("Time  | A    B    Cin | Sum  Cout | Expected");
        $display("-------------------------------------------");

        // Test Case 1: 2 + 3 + 0 = 5
        a = 4'd2; b = 4'd3; cin = 0;
        #10;
        $display("%0t  | %d    %d    %d   | %d    %d    | 5", $time, a, b, cin, sum, cout);

        // Test Case 2: 15 + 1 + 0 = 0 (Overflow/Carry)
        a = 4'd15; b = 4'd1; cin = 0;
        #10;
        $display("%0t  | %d   %d    %d   | %d    %d    | 0 (Cout=1)", $time, a, b, cin, sum, cout);

        // Test Case 3: 10 + 10 + 1 = 21 (5 with Carry)
        a = 4'd10; b = 4'd10; cin = 1;
        #10;
        $display("%0t  | %d   %d   %d   | %d    %d    | 5 (Cout=1)", $time, a, b, cin, sum, cout);
        
        $finish;
    end
endmodule
