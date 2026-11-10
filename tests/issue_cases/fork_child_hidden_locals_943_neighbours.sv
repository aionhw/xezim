// #943 neighbours: shared and hidden variables side by side in fork children.
typedef struct        { int a, b; } payload_t;
typedef struct packed { logic [7:0] x, y; } geom_t;

module top;
  logic clk = 0;
  initial repeat (60) #1 clk = ~clk;

  // U: one sibling hides `payload` in an inner block while the other writes
  // the shared `payload`; after the block the first sees the write.
  task automatic u_parent();
    int payload;
    payload = 1;
    fork
      begin
        begin
          string payload;
          payload = "h";
          repeat (3) @(posedge clk);
          $display("T|U s1 inner %s", payload);
        end
        $display("T|U s1 outer %0d", payload);
        payload = payload + 100;
      end
      begin
        repeat (2) begin
          @(posedge clk);
          payload += 1;
          $display("T|U s2 shared=%0d", payload);
        end
      end
    join
    $display("T|U parent payload=%0d", payload);
  endtask

  // Q: three views of one frame -- the parent's int and two children's own.
  task automatic q_parent();
    int payload;
    payload = 1;
    fork
      begin
        geom_t payload;
        payload = 16'h0a0b;
        repeat (2) begin @(posedge clk); payload.y++; $display("T|Q c1 %h", payload); end
      end
      begin
        payload_t payload;
        payload.a = 3; payload.b = 4;
        repeat (2) begin
          @(posedge clk);
          payload.a *= 2;
          $display("T|Q c2 a=%0d b=%0d", payload.a, payload.b);
        end
      end
      begin
        repeat (2) begin @(posedge clk); payload += 1; $display("T|Q c3 shared=%0d", payload); end
      end
    join
    $display("T|Q parent payload=%0d", payload);
  endtask

  // R: a grandchild hides the child's local of the same name, which hides the parent's.
  task automatic r_parent();
    int payload;
    payload = 7;
    fork
      begin
        string payload;
        payload = "mid";
        fork
          begin
            int payload;
            payload = 70;
            repeat (2) begin @(posedge clk); payload++; $display("T|R grandchild %0d", payload); end
          end
        join_none
        repeat (2) begin
          @(posedge clk);
          payload = {payload, "."};
          $display("T|R child %s", payload);
        end
      end
    join_none
    repeat (3) begin @(negedge clk); payload--; $display("T|R parent %0d", payload); end
  endtask

  // S: one-frame block shadowing (int outer, string inner) while another
  // process runs a geom_t `payload`.
  task automatic s_task();
    int payload;
    payload = 3;
    begin
      string payload;
      payload = "in";
      @(posedge clk);
      payload = {payload, "+"};
      $display("T|S inner %s %0d", payload, payload.len());
    end
    @(posedge clk);
    payload = payload * 5;
    $display("T|S outer %0d %0d", payload, $bits(payload));
  endtask
  task automatic s_other();
    geom_t payload;
    payload = 16'h1122;
    repeat (3) begin @(posedge clk); payload.x++; $display("T|S other %h", payload); end
  endtask

  initial begin #30; u_parent(); end
  initial begin #9; q_parent(); end
  initial begin #15; r_parent(); end
  initial begin #23; fork s_task(); s_other(); join end
endmodule
