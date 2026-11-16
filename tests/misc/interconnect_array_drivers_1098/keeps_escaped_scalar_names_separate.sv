// IEEE 1800-2023 §5.6.1: escaped identifiers contain printable non-whitespace characters; §7.4.2 array elements are separate objects. Each scalar has one driver, 7 or 9 (§6.6.7). §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets.
// Registering an interconnect element must not overwrite escaped scalar objects sharing its printed spelling; main-wrong array reader is omitted.
`timescale 1ns/1ps
nettype real N;
module source(output N n);assign n=2.0;endmodule
module reader(input N n);endmodule
module top;
 interconnect a[1];N \a[0] ;N \a[5] ;
 assign \a[0] =7.0;assign \a[5] =9.0;
 source s(a[0]);reader r(a[0]);
 initial begin #1;$display("T|escaped %g %g",\a[0] ,\a[5] );$finish;end
endmodule
