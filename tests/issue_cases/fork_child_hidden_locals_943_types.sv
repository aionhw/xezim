// #943: one local name in two live frames, different types; each frame keeps its own type.
typedef struct        { int a, b; } payload_t;
typedef struct packed { logic [7:0] x, y; } geom_t;
class pkt_c; int f; endclass

module top;
  logic clk = 0;
  initial repeat (20) #1 clk = ~clk;

  // A: parent int, fork child string (child stays live across edges)
  task automatic a_parent(int k);
    int payload;
    payload = 40 + k;
    fork
      begin
        string payload;
        payload = "child";
        repeat (3) begin
          @(posedge clk);
          payload = {payload, "x"};
          $display("T|A%0d child payload=%s len=%0d", k, payload, payload.len());
        end
      end
    join_none
    repeat (2) begin
      @(posedge clk);
      payload = payload + 1;
      $display("T|A%0d parent payload=%0d bits=%0d", k, payload, $bits(payload));
    end
  endtask

  // B: parent unpacked struct, child packed struct
  task automatic b_parent();
    payload_t payload;
    payload.a = 7; payload.b = 9;
    fork
      begin
        geom_t payload;
        payload.x = 8'h12; payload.y = 8'h34;
        repeat (3) begin
          @(posedge clk);
          payload.y = payload.y + 1;
          $display("T|B child x=%h y=%h whole=%h bits=%0d",
                   payload.x, payload.y, payload, $bits(payload));
        end
      end
    join_none
    repeat (2) begin
      @(posedge clk);
      payload.b = payload.b + 1;
      $display("T|B parent a=%0d b=%0d", payload.a, payload.b);
    end
  endtask

  // C: parent class handle, child struct
  task automatic c_parent();
    pkt_c payload;
    payload = new(); payload.f = 5;
    fork
      begin
        payload_t payload;
        payload.a = 100; payload.b = 200;
        repeat (3) begin
          @(posedge clk);
          payload.a++;
          $display("T|C child a=%0d b=%0d", payload.a, payload.b);
        end
      end
    join_none
    repeat (2) begin
      @(posedge clk);
      payload.f++;
      $display("T|C parent f=%0d", payload.f);
    end
  endtask

  // D: nested live frames (outer struct, inner packed), outer reads after inner returns
  task automatic d_inner();
    geom_t payload;
    payload = 16'hA1B2;
    repeat (2) begin
      @(posedge clk);
      payload.x++;
      $display("T|D inner x=%h y=%h bits=%0d", payload.x, payload.y, $bits(payload));
    end
  endtask
  task automatic d_outer();
    payload_t payload;
    payload.a = 1; payload.b = 2;
    d_inner();
    @(posedge clk);
    payload.b += 10;
    $display("T|D outer a=%0d b=%0d", payload.a, payload.b);
  endtask

  // E: two independent processes, same-named locals of different types
  task automatic e_str();
    string payload;
    payload = "s";
    repeat (3) begin
      @(posedge clk);
      payload = {payload, "t"};
      $display("T|E str payload=%s", payload);
    end
  endtask
  task automatic e_geom();
    geom_t payload;
    payload = 16'h0102;
    repeat (3) begin
      @(posedge clk);
      payload.y = payload.y + 2;
      $display("T|E geom x=%h y=%h", payload.x, payload.y);
    end
  endtask

  initial begin a_parent(0); end
  initial begin #3; b_parent(); end
  initial begin #5; c_parent(); end
  initial begin #7; d_outer(); end
  initial begin #9; fork e_str(); e_geom(); join end
endmodule
