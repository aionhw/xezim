`timescale 1ns/1ns

typedef logic [1:0] u2_t ;
typedef logic [7:0] u8_t ;

interface test_if (input logic clk, input logic rst_l);

	mailbox #(u8_t) ping_mbox ;

	initial begin
		ping_mbox = new();
	end

endinterface: test_if

module Sender0 (input logic clk, input logic rst_l, test_if test_intf);

	initial begin
		u8_t s0_val ;
		forever begin
			repeat(3) @(posedge clk iff rst_l);
			void'(std::randomize(s0_val) with { s0_val[0] == 0 ; });
			test_intf.ping_mbox.put(s0_val);
		end
	end

endmodule: Sender0

module Sender1 (input logic clk, input logic rst_l, test_if test_intf);

	initial begin
		u8_t s1_val ;
		forever begin
			repeat(5) @(posedge clk iff rst_l);
			void'(std::randomize(s1_val) with { s1_val[0] == 1 ; });
			test_intf.ping_mbox.put(s1_val);
		end
	end


endmodule: Sender1

module Receiver0 (input logic clk, input logic rst_l, input int num_cycles, test_if test_intf);

	initial begin
		u8_t recv_val ;
		forever @(posedge clk iff rst_l) begin
			test_intf.ping_mbox.get(recv_val);
			$display("c%0d: Received: %0d", num_cycles, recv_val);
		end
	end

endmodule: Receiver0


module testbench ;

	
	logic clk, rst_l ;
	int num_cycles ;

	test_if test_intf (.clk(clk),.rst_l(rst_l));

	Sender0 Sender0Inst (.*);
	Sender1 Sender1Inst (.*);
	Receiver0 Receiver0Inst (.*);

	initial begin
		fork
			// Clock generation block
			begin
				clk = 1'b0 ;
				// 100 MHz = 10ns time period
				forever #5 clk = ~clk ;
			end
			// Reset generation block
			begin
				rst_l = 1'b0 ;
				repeat (20) @(posedge clk);
				rst_l = 1'b1 ;
			end
			// End of test block
			begin
				@(posedge clk iff rst_l);
				repeat (1000) @(posedge clk);
				$finish(1);
			end
			// Track number of test cycles
			begin
				num_cycles = 0 ;
				@(posedge clk iff rst_l);
				forever @(posedge clk) begin
					num_cycles ++ ;
				end
			end
		join
	end




endmodule: testbench