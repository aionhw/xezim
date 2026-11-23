module packed_loop_check;
  localparam int entry = 0;
  typedef struct packed { logic lock; logic [5:0] value; } control_t;
  typedef struct packed { control_t csr; } entry_t;
  typedef struct packed { entry_t [7:0] entries; } bank_t;
  bank_t wr;
  logic [47:0] words;
  logic [7:0] bits;
  always_comb begin
    for (int entry=0; entry<8; entry++) begin
      wr.entries[entry].csr.lock=bits[entry];
      wr.entries[entry].csr.value=words[entry*6 +: 6];
    end
  end
  initial begin
    words=48'h9876543210ab; bits=8'ha3; #1;
    $display("first=%h last=%h bank=%h",wr.entries[0].csr,wr.entries[7].csr,wr);
    words=~words; bits=~bits; #1;
    $display("first=%h last=%h bank=%h",wr.entries[0].csr,wr.entries[7].csr,wr);
    $finish;
  end
endmodule
