//! IEEE 1800-2017 §8.25, §6.20: a subroutine local of a parameterized class
//! sized by a class parameter or localparam (`bit [N-1:0] x;`, the key of
//! `bit seen[bit [N-1:0]]`) takes the executing specialization's width; it
//! was sized as if `N` were unknown (#681, reported by Ganesh-TNL).
use xezim::simulate;

fn shown(src: &str) -> Vec<String> {
    simulate(src, 10_000)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("R ") || m.starts_with("TEST_") || m.starts_with("CHECK_FAIL"))
        .collect()
}

#[test]
fn reported_testbench() {
    assert_eq!(
        shown(
            r#"
module tb;
   int errors, checks;
   task automatic chk(input int id, input int got, input int exp);
      checks++;
      if (got !== exp) begin
         errors++;
         $display("CHECK_FAIL id=%0d got=%0d exp=%0d", id, got, exp);
      end
   endtask

   class rc #(parameter int N = 16);
      // task-local assoc array with a parameterized key
      task automatic fill(input int K, output int u);
         bit seen[bit [N-1:0]];
         u = 0;
         for (int i=0;i<K;i++)
            if (!seen.exists(i)) begin seen[i]=1; u++; end
      endtask
      // function-local packed variable with a parameterized width
      function automatic int var_allones();
         bit [N-1:0] x;
         x = '1;
         return x;
      endfunction
      // function-local packed array with a parameterized width
      function automatic int arr_allones();
         bit [N-1:0] a[4];
         int s = 0;
         for (int i=0;i<4;i++) begin a[i]='1; s += a[i]; end
         return s;
      endfunction
   endclass

   initial begin
      rc #(.N(8))  a8;
      rc #(.N(16)) a16;
      rc #(.N(17)) a17;
      rc #(.N(20)) a20;
      int u;
      a8=new(); a16=new(); a17=new(); a20=new();

      // task-local assoc: must retain K distinct keys
      a8.fill(8, u);   chk(1, u, 8);
      a16.fill(16, u); chk(2, u, 16);
      a17.fill(17, u); chk(3, u, 17);

      // function-local packed var: '1 must be 2^N-1
      chk(4, a8.var_allones(),  255);
      chk(5, a16.var_allones(), 65535);
      chk(6, a20.var_allones(), 1048575);

      // function-local packed array: 4 * (2^N-1)
      chk(7, a8.arr_allones(),  1020);
      chk(8, a16.arr_allones(), 262140);

      if (errors == 0) $display("TEST_PASS");
      else             $display("TEST_FAIL: %0d CHECKS", errors);
      $display("summary: %0d checks, %0d failed", checks, errors);
      $finish;
   end
endmodule


"#
        ),
        ["TEST_PASS"]
    );
}

/// The width follows each specialization, and a plain class's localparam.
#[test]
fn local_widths_per_specialization() {
    assert_eq!(
        shown(
            r#"class rc #(parameter int N = 16);
  function automatic int w(); bit [N-1:0] x; x = '1; return x; endfunction
  function automatic int b(); bit [N-1:0] x; return $bits(x); endfunction
  function automatic int p(); return N; endfunction
endclass
class plain;
  localparam int M = 8;
  function automatic int w(); bit [M-1:0] x; x = '1; return x; endfunction
endclass
module t;
  initial begin
    rc #(8) a = new; rc #(20) c = new; plain q = new;
    $display("R %0d %0d %0d | %0d %0d %0d | %0d", a.w(), a.b(), a.p(), c.w(), c.b(), c.p(), q.w());
  end
endmodule
"#
        ),
        ["R 255 8 8 | 1048575 20 20 | 255"]
    );
}
