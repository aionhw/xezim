// SPDX-License-Identifier: MIT
//
// probe_struct_nettype.sv — multi-driver probe for struct-typed nettype
// resolution with `real` field. The resolver sums `field1` (real addition)
// and ORs `field2` (logical OR, not AND), so we can see what happens when
// the resolver's input queue has multiple drivers.

typedef struct {
  real    field1;
  bit     field2;
} T;

function automatic T Tsum (input T driver []);
  T result;
  result.field1 = 0.0;
  result.field2 = 1'b0;
  foreach (driver[i]) begin
    result.field1 += driver[i].field1;
    result.field2 |= driver[i].field2;
  end
  return result;
endfunction

nettype T wTsum with Tsum;

module probe;
  // Single driver cases
  wTsum a; assign a = '{1.5, 1'b1};
  wTsum b; assign b = '{2.5, 1'b0};

  // Multi-driver cases (resolver should sum/OR across all)
  wTsum m01; assign m01 = '{1.0, 1'b0}; assign m01 = '{2.0, 1'b1};
  // expected per resolver: field1 = 1.0 + 2.0 = 3.0, field2 = 0 | 1 = 1

  wTsum m11; assign m11 = '{1.0, 1'b1}; assign m11 = '{2.0, 1'b1};
  // expected: field1 = 3.0, field2 = 1 | 1 = 1

  wTsum m00; assign m00 = '{1.0, 1'b0}; assign m00 = '{2.0, 1'b0};
  // expected: field1 = 3.0, field2 = 0 | 0 = 0

  wTsum m3; assign m3 = '{1.0, 1'b1}; assign m3 = '{2.0, 1'b0}; assign m3 = '{3.0, 1'b1};
  // expected: field1 = 1.0+2.0+3.0 = 6.0, field2 = 1|0|1 = 1

  // Wide case: each bit driven independently via assignment pattern
  wTsum mw_a; assign mw_a = '{0.5, 1'b1}; assign mw_a = '{1.5, 1'b0};
  // expected: 0.5+1.5 = 2.0, 1|0 = 1

  initial begin
    #1;
    $display("Single-driver cases:");
    $display("  a = [1.5, 1] -> field1=%f field2=%b", a.field1, a.field2);
    $display("  b = [2.5, 0] -> field1=%f field2=%b", b.field1, b.field2);
    $display("Multi-driver cases:");
    $display("  m01 = [1.0,0] + [2.0,1] -> field1=%f field2=%b  (expected 3.0, 1)",
             m01.field1, m01.field2);
    $display("  m11 = [1.0,1] + [2.0,1] -> field1=%f field2=%b  (expected 3.0, 1)",
             m11.field1, m11.field2);
    $display("  m00 = [1.0,0] + [2.0,0] -> field1=%f field2=%b  (expected 3.0, 0)",
             m00.field1, m00.field2);
    $display("  m3  = [1.0,1] + [2.0,0] + [3.0,1] -> field1=%f field2=%b  (expected 6.0, 1)",
             m3.field1, m3.field2);
    $display("  mw_a = [0.5,1] + [1.5,0] -> field1=%f field2=%b  (expected 2.0, 1)",
             mw_a.field1, mw_a.field2);
  end
endmodule