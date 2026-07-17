//! Hard-tail ivtest behaviors: representative reductions of the remaining
//! ivtest failures (multi-dim packed selects, package scoping, time literals,
//! ...). Each asserts the self-checking "PASSED" marker.

use xezim::simulate;

fn passes(src: &str) -> bool {
    match simulate(src, 100_000) {
        Ok(sim) => {
            let out: String = sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n");
            out.contains("PASSED") && !out.contains("FAILED")
        }
        Err(_) => false,
    }
}

/// §11.4.1: an lvalue index with a side effect (`x[i++] = v`,
/// `x[i++] += 2`) evaluates the index exactly once (ivtest pr3390385).
#[test]
fn lvalue_index_side_effect_single_eval() {
    assert!(passes(r#"
module tb;
reg [1:0] i, j;
reg [3:0] x[0:2];
reg error;
initial begin
   error = 0;
   i = 0;
   j = i++;
   if (i !== 2'b01 || j !== 2'b00) error = 1;
   i = 0;
   x[0] = 4'dx; x[1] = 4'dx;
   x[i++] = 0;
   if (x[0] !== 4'd0 || x[1] !== 4'dx || i !== 2'd1) error = 1;
   i = 0;
   x[0] = 1;
   x[i++] += 2;
   if (x[0] !== 4'd3) error = 1;
   if (i !== 2'd1) error = 1;
   if (error == 0) $display("PASSED"); else $display("FAILED");
end
endmodule
"#));
}

/// §7.4.1: nested index on a 3-D packed vector selects the element slice,
/// with $bits agreeing at each level (ivtest br_gh112a).
#[test]
fn packed_3d_nested_index_descending() {
    assert!(passes(r#"
module t;
reg [1:0][15:0][7:0] array;
reg failed = 0;
integer i;
reg [3:0] index;
initial begin
  if ($bits(array) !== 256) failed = 1;
  if ($bits(array[0]) !== 128) failed = 1;
  if ($bits(array[0][0]) !== 8) failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    array[0][index] = {4'd0, index};
    array[1][index] = {4'd1, index};
  end
  if (array !== 256'h1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100)
    failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    if (array[0][index] !== {4'd0, index}) failed = 1;
    if (array[1][index] !== {4'd1, index}) failed = 1;
  end
  if (failed) $display("FAILED"); else $display("PASSED");
end
endmodule
"#));
}

/// §7.4.1: ascending packed ranges label the LEFT bound as the
/// most-significant element (ivtest br_gh112b).
#[test]
fn packed_3d_nested_index_ascending() {
    assert!(passes(r#"
module t;
reg [0:1][0:15][0:7] array;
reg failed = 0;
integer i;
reg [3:0] index;
initial begin
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    array[0][index] = {4'd0, index};
    array[1][index] = {4'd1, index};
  end
  if (array !== 256'h000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f)
    failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    if (array[0][index] !== {4'd0, index}) failed = 1;
    if (array[1][index] !== {4'd1, index}) failed = 1;
  end
  if (failed) $display("FAILED"); else $display("PASSED");
end
endmodule
"#));
}

/// §7.4.1: non-zero-based and negative packed bounds normalize per
/// dimension, with signed index expressions (ivtest br_gh112c/e/f).
#[test]
fn packed_3d_nested_index_offset_and_negative_bounds() {
    assert!(passes(r#"
module t;
reg [2:1][16:1][8:1] a;
reg [0:-1][14:-1][6:-1] b;
reg failed = 0;
integer i;
reg [3:0] index;
reg signed [4:0] sindex;
initial begin
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    a[1][index+16'd1] = {4'd0, index};
    a[2][index+16'd1] = {4'd1, index};
    sindex = i[4:0];
    b[-1][-5'sd1+sindex] = {4'd0, sindex[3:0]};
    b[ 0][-5'sd1+sindex] = {4'd1, sindex[3:0]};
  end
  if (a !== 256'h1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100)
    failed = 1;
  if (b !== 256'h1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100)
    failed = 1;
  for (i = 0; i < 16; i++) begin
    index = i[3:0];
    if (a[1][index+16'd1] !== {4'd0, index}) failed = 1;
    if (a[2][index+16'd1] !== {4'd1, index}) failed = 1;
  end
  if (failed) $display("FAILED"); else $display("PASSED");
end
endmodule
"#));
}

/// §7.4.2: writes and reads through a packed array of packed struct nested
/// inside another packed struct — `main.sub_list[i].f` shares the parent's
/// storage (ivtest gh161a/gh161b).
#[test]
fn nested_packed_struct_array_member_rw() {
    assert!(passes(r#"
module test();
   typedef struct packed { logic [31:0] sub_local; } row_entry_t;
   typedef struct packed {
      logic [31:0] row_local;
      row_entry_t         sub;
      row_entry_t [1:0]   sub_list;
   } row_t;
   row_t main;
   initial begin
      main.row_local = 32'hCAFE;
      main.sub.sub_local = 32'h00000001;
      main.sub_list[0].sub_local = 32'hACE;
      main.sub_list[1].sub_local = 32'hECA;
      if (main !== 128'h0000cafe0000000100000eca00000ace) begin
         $display("FAILED -- main"); $finish;
      end
      if (main.sub.sub_local !== 32'h00000001) begin
         $display("FAILED -- sub.sub_local"); $finish;
      end
      if (main.sub_list[0].sub_local !== 32'hACE) begin
         $display("FAILED -- sub_list[0]"); $finish;
      end
      if (main.sub_list[1].sub_local !== 32'hECA) begin
         $display("FAILED -- sub_list[1]"); $finish;
      end
      $display("PASSED");
   end
endmodule
"#));
}

/// §8.12 vs constructor call: `foo[i] = new(i)` with an INT arg constructs —
/// it must never be read as the shallow-copy form `new <src>` just because
/// the int's value collides with a live heap index (ivtest sv_foreach3/4).
#[test]
fn class_new_int_arg_is_construction_not_copy() {
    assert!(passes(r#"
module main;
   class test_t;
      reg [7:0] a;
      function new (int ax); a = ax; endfunction
   endclass
   class container_t;
      test_t foo [0:3];
      task run();
	 test_t tmp;
	 for (int i = 0 ; i < 4 ; i++) foo[i] = new(i);
	 for (int i = 0 ; i < 4 ; i++) begin
	    tmp = foo[i];
	    if (tmp == null) begin $display("FAILED -- null %0d", i); $finish; end
	    if (tmp.a !== 8'(i)) begin $display("FAILED -- a %0d %0d", i, tmp.a); $finish; end
	 end
	 $display("PASSED");
      endtask
   endclass
   container_t dut;
   initial begin dut = new; dut.run; end
endmodule
"#));
}

/// Multi-dim fixed class-array member: element writes/reads (`foo[i][j]`)
/// and multi-var `foreach (foo[ia,ib])` resolve per-instance storage —
/// previously only the first dimension was recorded (ivtest sv_foreach3/4).
#[test]
fn class_member_2d_array_rw_and_foreach() {
    assert!(passes(r#"
module main;
   class test_t;
      reg [1:0] a;
      reg [2:0] b;
      function new (int ax, int bx); a = ax; b = bx; endfunction
   endclass
   class container_t;
      test_t foo [0:3][0:7];
      function new();
	 for (int i = 0 ; i < 4 ; i++)
	    for (int j = 0 ; j < 8 ; j++)
	       foo[i][j] = new(i,j);
      endfunction
      task run();
	 test_t tmp;
	 foreach (foo[ia,ib]) begin
	    if (ia > 3 || ib > 7) begin
	       $display("FAILED -- range ia=%0d ib=%0d", ia, ib); $finish;
	    end
	    tmp = foo[ia][ib];
	    if (tmp.a !== ia[1:0] || tmp.b !== ib[2:0]) begin
	       $display("FAILED -- foo[%0d][%0d] = %b", ia, ib, {tmp.a, tmp.b}); $finish;
	    end
	    foo[ia][ib] = null;
	 end
	 for (int i = 0 ; i < 4 ; i++)
	    for (int j = 0 ; j < 8 ; j++)
	       if (foo[i][j] != null) begin
		  $display("FAILED -- not visited %0d %0d", i, j); $finish;
	       end
	 $display("PASSED");
      endtask
   endclass
   container_t dut;
   initial begin dut = new; dut.run; end
endmodule
"#));
}

/// §12.7.3: a foreach loop var BEYOND the unpacked dims iterates the
/// element's packed dimension (ivtest sv_foreach5).
#[test]
fn foreach_packed_dim_loop_var() {
    assert!(passes(r#"
module test();
reg [3:0] array[0:1][0:2];
reg [3:0] expected;
reg failed = 0;
initial begin
  for (int i = 0; i < 2; i++)
    for (int j = 0; j < 3; j++)
      array[i][j] = i * 4 + j;
  foreach (array[i,j,k]) begin
    expected = i * 4 + j;
    if (array[i][j][k] !== expected[k]) failed = 1;
  end
  foreach (array[i,j]) begin
    expected = i * 4 + j;
    if (array[i][j] !== expected) failed = 1;
  end
  if (failed) $display("FAILED"); else $display("PASSED");
end
endmodule
"#));
}

/// §7.4.1: genloop port connections onto elements of a multi-D PACKED net
/// (`wire logic [3:0][7:0] foo; test dut(.sum(foo[idx]))`) drive the full
/// element slice, and `foo[i]` reads a slice, not a bit (ivtest packeda2).
#[test]
fn packed_net_elem_port_connection() {
    assert!(passes(r#"
module main;
   wire logic [3:0][7:0] foo;
   genvar idx;
   for (idx = 0 ; idx <= 3 ; idx = idx+1) begin: test
      test dut (.sum(foo[idx]), .a(idx));
   end
   logic [7:0] tmp;
   initial begin
      #1;
      for (tmp = 0 ; tmp <= 3 ; tmp = tmp+1) begin
	 if (foo[tmp] !== (tmp+8'd5)) begin
	    $display("FAILED -- foo[%d] = %b", tmp, foo[tmp]);
	    $finish;
	 end
      end
      $display("PASSED");
   end
endmodule
module test (output logic[7:0] sum, input logic [7:0]a);
   assign sum = a + 8'd5;
endmodule
"#));
}

/// §6.6.1: an undriven net reads Z, not X — and bits of a net nothing
/// drives stay z after partial continuous assigns (ivtest
/// struct_packed_write_read2's word_se0/sw0/ep0 checks).
#[test]
fn undriven_net_defaults_to_z() {
    assert!(passes(r#"
module main;
   typedef struct packed {
      logic [7:0] high;
      logic [7:0] low;
   } word_t;
   wire word_t word_se0;
   wire word_t word_ep1;
   assign word_ep1.high [3:0] = 4'b1111;
   assign word_ep1.low  [3:0] = 4'b0000;
   initial begin
      #1;
      if (word_se0 !== 16'bzzzzzzzz_zzzzzzzz) begin
	 $display("FAILED -- word_se0 = 'b%b", word_se0); $finish;
      end
      if (word_ep1 !== 16'bzzzz1111_zzzz0000) begin
	 $display("FAILED -- word_ep1 = 'b%b", word_ep1); $finish;
      end
      $display("PASSED");
   end
endmodule
"#));
}

/// Nested cont-assign selects on a multi-D packed NET write the addressed
/// slice with full element width (regression probe for the interpreted
/// ContAssign path: infer_lhs_width must not truncate to 1 bit).
#[test]
fn packed_net_nested_index_cont_assign() {
    assert!(passes(r#"
module main;
   wire logic [1:0][3:0][7:0] foo;
   assign foo[0][0] = 8'hA5;
   assign foo[0][3] = 8'hC3;
   assign foo[1][2] = 8'h7E;
   initial begin
      #1;
      if (foo !== 64'hzz7ezzzzc3zzzza5) begin
	 $display("FAILED -- foo=%h", foo); $finish;
      end
      $display("PASSED");
   end
endmodule
"#));
}
