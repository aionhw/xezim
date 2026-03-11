
module traffic_light (
    input  logic clk,
    input  logic rst_n,    // Active-low reset
    output logic [2:0] lights // [Red, Yellow, Green]
);

    // Define states using an enumerated type
    typedef enum logic [1:0] {
        GREEN  = 2'b00,
        YELLOW = 2'b01,
        RED    = 2'b10
    } state_t;

    state_t current_state, next_state;

    // 1. State Register (Sequential)
    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n)
            current_state <= RED;
        else
            current_state <= next_state;
    end

    // 2. Next State Logic (Combinational)
    always_comb begin
        case (current_state)
            GREEN:  next_state = YELLOW;
            YELLOW: next_state = RED;
            RED:    next_state = GREEN;
            default: next_state = RED;
        endcase
    end

    // 3. Output Logic
    assign lights = (current_state == GREEN)  ? 3'b001 :
                    (current_state == YELLOW) ? 3'b010 :
                    (current_state == RED)    ? 3'b100 : 3'b100;

endmodule

module tb_traffic_light();
    logic clk;
    logic rst_n;
    logic [2:0] lights;

    // Instantiate Unit Under Test [cite: 11]
    traffic_light uut (
        .clk(clk),
        .rst_n(rst_n),
        .lights(lights)
    );

    // Robust Clock Generation
    initial clk = 0;
    always #5 clk = ~clk; 

    // Improved Stimulus Block
    initial begin
        $display("Starting Simulation...");
        $display("Time\t State (R Y G)");
        
        rst_n = 0;           // Assert reset 
        #12 rst_n = 1;       // Release reset slightly after an edge to avoid races

        repeat (8) begin
            @(posedge clk);
            // Use #1 delay to see the value AFTER the flip-flop updates
            #1 $display("%0t\t %b", $time, lights); 
        end

        $display("Simulation Finished.");
        $finish; 
    end
endmodule



