// IEEE 1800-2023 §7.2: a member may have "a default value" (module values 1.25,7); §§7.4.2,10.5,23.6 give direct element reads the explicitly assigned 2.5/2 and 4.5/4.
// Implicit generated defaults and multidimensional continuous mirrors remain open; module defaults and direct structure-array reads already work.
`timescale 1ns/1ps
typedef struct {real V=1.25;int tag=7;} E;
module top;
 E module_default;
 if(1)begin:g
 E explicit_default='{1.25,7};E a[2];E m[2][2];
 initial begin #1;a[1]='{2.5,2};m[1][1]='{4.5,4};end
 end
 initial begin #2;
 $display("T|defaults %0.2f %0d",module_default.V,module_default.tag);
 $display("T|explicit %0.2f %0d",g.explicit_default.V,g.explicit_default.tag);
 $display("T|arrays %0.1f %0d %0.1f %0d",g.a[1].V,g.a[1].tag,g.m[1][1].V,g.m[1][1].tag);$finish;end
endmodule
