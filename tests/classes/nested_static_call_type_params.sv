module top;
  // IEEE 1800-2023 §8.25: both value parameters of a nested class
  // specialization must bind in the enclosing method's specialization.
  class cfg #(int AW = 32, int DW = 32);
    int tag = 7;
  endclass
  class db #(type T = int);
    static T value;
    static function void set(T v); value = v; endfunction
    static function T get(); return value; endfunction
  endclass
  class agent #(int AW = 32, int DW = 32);
    function cfg#(AW,DW) get_cfg(); return db#(cfg#(AW,DW))::get(); endfunction
  endclass
  cfg#(12,32) sent, got;
  cfg#(49,128) other_sent, other_got;
  agent#(12,32) a;
  agent#(49,128) b;
  initial begin
    sent = new; other_sent = new;
    a = new; b = new;
    db#(cfg#(12,32))::set(sent);
    db#(cfg#(49,128))::set(other_sent);
    got = a.get_cfg(); other_got = b.get_cfg();
    if (got != sent || other_got != other_sent)
      $fatal(1, "nested class parameter specialization lost or aliased");
    $display("NESTED_SPEC_PASS");
    $finish;
  end
endmodule
