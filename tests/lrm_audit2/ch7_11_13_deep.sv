// Deeper cuts: queue methods, streaming slices, task semantics
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[d] %s", name); fails++; end
  task automatic tconst(const ref int arr[3], output int sum);
    sum = arr[0] + arr[1] + arr[2];
  endtask
  initial begin
    begin // queue methods deep
      int q[$] = {5, 1, 4, 1, 3};
      int idx[$]; int r[$];
      r = q.find_last_index with (item == 1);
      `CK("find_last_index", r.size() == 1 && r[0] == 3)
      r = q.find_index with (item > 2);
      `CK("find_index multi", r.size() == 3)
      q.sort() with (item);
      `CK("sort with", q[0] == 1 && q[4] == 5)
      begin
        int qq[$] = {3, 1, 2};
        int total;
        total = qq.sum() with (item * 2);
        `CK("sum with", total == 12)
        `CK("and-reduce via product", qq.product() == 6)
      end
    end
    begin // streaming with slice size
      logic [31:0] w;
      byte b[4];
      w = 32'h11223344;
      {>>byte{b}} = w;
      `CK("stream unpack bytes", b[0] == 8'h11 && b[3] == 8'h44)
      {<<byte{b}} = w;
      `CK("stream reverse unpack", b[0] == 8'h44 && b[3] == 8'h11)
      w = {<<4{16'hABCD}};
      `CK("nibble reverse", w[15:0] == 16'hDCBA)
    end
    begin // const ref
      int a3[3];
      int s;
      a3 = '{10, 20, 30};
      tconst(a3, s);
      `CK("const ref array arg", s == 60)
    end
    begin // wait fork
      int done;
      done = 0;
      fork
        #2 done = 1;
      join_none
      wait fork;
      `CK("wait fork", done == 1 && $time >= 2)
    end
    $display("CHD CHECKS DONE fails=%0d", fails);
  end
endmodule
