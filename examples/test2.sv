// First, define a single 1-bit Full Adder
module full_adder (
    input  logic a,
    input  logic b,
    input  logic cin,
    output logic sum,
    output logic cout
);
    assign sum  = a ^ b ^ cin;
    assign cout = (a & b) | (cin & (a ^ b));
endmodule

// Now, connect 4 of them together
module adder_4bit_struct (
    input  logic [3:0] a,
    input  logic [3:0] b,
    input  logic       cin,
    output logic [3:0] sum,
    output logic       cout
);
    logic c1, c2, c3; // Internal carry wires

    // Instantiate 4 Full Adders
    full_adder fa0 (.a(a[0]), .b(b[0]), .cin(cin), .sum(sum[0]), .cout(c1));
    full_adder fa1 (.a(a[1]), .b(b[1]), .cin(c1),  .sum(sum[1]), .cout(c2));
    full_adder fa2 (.a(a[2]), .b(b[2]), .cin(c2),  .sum(sum[2]), .cout(c3));
    full_adder fa3 (.a(a[3]), .b(b[3]), .cin(c3),  .sum(sum[3]), .cout(cout));

endmodule





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
