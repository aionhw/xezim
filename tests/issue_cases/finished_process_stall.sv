// IEEE 1800-2023 §9.3.2 / §9.4.1: a completed zero-delay child can
// still be named by the simulator's same-slot livelock diagnostic.
module top;
  initial begin
    fork begin repeat (4) #0; end join
    forever #0;
  end
endmodule
