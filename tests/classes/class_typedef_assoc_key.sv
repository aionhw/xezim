// §8.25, §6.18: an associative array keyed by a class typedef over a class
// parameter has the key width of each object's specialization.
class keyed #(int AW = 32);
  typedef logic [AW-1:0] a_t;
  logic [7:0] m[a_t];
endclass

module top;
  keyed kd;
  keyed #(8) k8, c8;
  keyed #(40) k40;
  initial begin
    kd = new;
    k8 = new;
    k40 = new;
    for (int i = 0; i < 4; i++) begin
      kd.m[32'h8000_0000 + i] = i;
      k40.m[40'h80_0000_0000 + i] = i;
    end
    k8.m[8'hff] = 1;
    k8.m[9'h100] = 2;
    $display("T|num|%0d %0d %0d", kd.m.num(), k8.m.num(), k40.m.num());
    c8 = new k8; c8.m[9'h100] = 3; $display("T|copy|%0d %0d", c8.m.num(), c8.m[0]);
    foreach (kd.m[k]) $display("T|kd_key|%0h", k);
    foreach (k8.m[k]) $display("T|k8_key|%0h", k);
    foreach (k40.m[k]) $display("T|k40_key|%0h", k);
  end
endmodule
