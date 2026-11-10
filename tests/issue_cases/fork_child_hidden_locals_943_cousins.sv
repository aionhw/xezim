// Cousin kinds of #943 with values: a parent task and its join_none child both
// declare `payload`; both stay live and print their own local.
typedef struct        { int a, b, c, d; } payload_t;
typedef struct packed { logic [7:0] x, y, z, w; } geom_t;
class pkt_c; int f; endclass

module top;
  logic clk = 0;
  initial repeat (16) #1 clk = ~clk;

  task automatic k1(int k);            // unpacked struct, same name
    payload_t payload;
    payload.a = k;
    fork begin
      payload_t payload;
      payload.b = k + 100;
      repeat (2) begin
        @(posedge clk);
        payload.b++;
        $display("T|k1.%0d child a=%0d b=%0d", k, payload.a, payload.b);
      end
    end join_none
    repeat (2) begin
      @(negedge clk);
      payload.a += 10;
      $display("T|k1.%0d parent a=%0d b=%0d", k, payload.a, payload.b);
    end
  endtask

  task automatic k2(int k);            // packed struct, same name
    geom_t payload;
    payload.x = k;
    fork begin
      geom_t payload;
      payload.y = k + 8'h40;
      repeat (2) begin @(posedge clk); payload.y++; $display("T|k2.%0d child %h", k, payload); end
    end join_none
    repeat (2) begin
      @(negedge clk);
      payload.x += 8'h10;
      $display("T|k2.%0d parent %h", k, payload);
    end
  endtask

  task automatic k3(int k);            // string, same name
    string payload;
    payload = "parent";
    fork begin
      string payload;
      payload = "child";
      repeat (2) begin
        @(posedge clk);
        payload = {payload, "+"};
        $display("T|k3.%0d child %s %0d", k, payload, payload.len());
      end
    end join_none
    repeat (2) begin
      @(negedge clk);
      payload = {payload, "-"};
      $display("T|k3.%0d parent %s %0d", k, payload, payload.len());
    end
  endtask

  task automatic k4(int k);            // class handle, same name
    pkt_c payload;
    payload = new(); payload.f = k;
    fork begin
      pkt_c payload;
      payload = new(); payload.f = k + 50;
      repeat (2) begin
        @(posedge clk);
        payload.f++;
        $display("T|k4.%0d child f=%0d", k, payload.f);
      end
    end join_none
    repeat (2) begin
      @(negedge clk);
      payload.f += 3;
      $display("T|k4.%0d parent f=%0d", k, payload.f);
    end
  endtask

  task automatic k8(int k);            // int, same name
    int payload;
    payload = k;
    fork begin
      int payload;
      payload = k + 1000;
      repeat (2) begin @(posedge clk); payload++; $display("T|k8.%0d child %0d", k, payload); end
    end join_none
    repeat (2) begin @(negedge clk); payload += 7; $display("T|k8.%0d parent %0d", k, payload); end
  endtask

  task automatic kx(int k);            // int parent, string child
    int payload;
    payload = k;
    fork begin
      string payload;
      payload = "s";
      repeat (2) begin
        @(posedge clk);
        payload = {payload, "t"};
        $display("T|kx.%0d child %s", k, payload);
      end
    end join_none
    repeat (2) begin
      @(negedge clk);
      payload += 7;
      $display("T|kx.%0d parent %0d %0d", k, payload, $bits(payload));
    end
  endtask

  initial begin
    fork k1(1); k1(2); k2(1); k3(1); k4(1); k8(1); k8(2); kx(1); join
  end
endmodule
