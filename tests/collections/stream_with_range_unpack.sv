// IEEE 1800-2023 §11.4.14.4: the with [ array_range ] of a stream operand is
// evaluated immediately before its array is streamed; packing past the end of
// the array streams nonexistent entries; unpacking resizes a dynamic array or
// queue to the range; a continuous assignment re-evaluates when a bound changes.
class Packet;
  int header;
  int len;
  byte payload[];
  int crc;
endclass
module top;
  bit [7:0] a[4], q[$], dyn[], dq[$];
  byte unsigned s[$];
  bit [55:0] w;
  bit [15:0] cs, cc;
  bit [47:0] wide;
  byte stream[$];
  int lo = 1, hi = 2;
  // §10.3.2, §9.2.2.2.1: the bounds are operands of the expression.
  assign cs = {>>{a with [lo:hi]}};
  always_comb cc = {>>{a with [lo+:2]}};
  initial begin
    Packet p, r;
    a = '{8'h11, 8'h22, 8'h33, 8'h44};
    q = '{8'h11, 8'h22, 8'h33, 8'h44};
    #1;
    $display("T|cont %h comb %h", cs, cc);
    lo = 0; hi = 1;
    #1;
    $display("T|cont2 %h comb2 %h", cs, cc);
    wide = {>>{q with [0:5]}};
    $display("T|beyond %h", wide);
    // Past the end of a queue and of a fixed array: nonexistent entries.
    w = {>>{q with [0:5], 8'hff}};
    $display("T|beyond_queue %h", w);
    w = {>>{a with [2:5], 8'hff}};
    $display("T|beyond_fixed %h", w);
    s = {>>{q with [2:4]}};
    $display("T|beyond_size %0d %h %h %h", s.size(), s[0], s[1], s[2]);
    dyn = new[1];
    dyn[0] = 8'h99;
    {>>{dyn with [1:2]}} = 16'h0102;
    $display("T|dynunpack size=%0d %h %h %h", dyn.size(), dyn[0], dyn[1], dyn[2]);
    {>>{dq with [0+:3]}} = 24'h0a0b0c;
    $display("T|queueunpack size=%0d %h %h %h", dq.size(), dq[0], dq[1], dq[2]);
    p = new; p.header = 32'h01020304; p.len = 3; p.payload = '{8'haa, 8'hbb, 8'hcc}; p.crc = 32'h0a0b0c0d;
    stream = {<< byte {p.header, p.len, p.payload with [0 +: p.len], p.crc}};
    $display("T|packet_pack size=%0d", stream.size());
    r = new;
    {<< byte {r.header, r.len, r.payload with [0 +: r.len], r.crc}} = stream;
    $display("T|packet_unpack %h %0d %0d %h %h %h %h", r.header, r.len, r.payload.size(), r.payload[0], r.payload[1], r.payload[2], r.crc);
    $finish;
  end
endmodule
