// Macros defined in an `include file; their bodies carry intra-assignment delays.
`define DELAYED_NBA(lhs, d, rhs) lhs <= #(d) rhs;
`define DELAYED_BLK(lhs, d, rhs) lhs = #(d) rhs;
