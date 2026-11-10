//! IEEE 1800-2023 §26.3: local header parameters shadow wildcard candidates,
//! while package declarations retain their lexical parameter bindings.

fn check(src: &str) {
    let sim = xezim::simulate(src, 100).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
    assert!(
        !sim.output.iter().any(|o| o.message.contains("Fatal")),
        "{:?}",
        sim.output
    );
}

#[test]
fn header_parameter_and_package_subroutines_keep_their_own_values() {
    check(
        r#"
package cfg;
 parameter int W=8;
 typedef logic [W-1:0] word_t;
 function automatic logic [W-1:0] echo(input logic [W-1:0] x);
   return x;
 endfunction
 function automatic int get_w(); return W; endfunction
 task automatic capture(output int value); value=W; endtask
 class Root; endclass
 class Reader extends Root;
   function int width(); return W; endfunction
   function logic [W-1:0] mask(); return '1; endfunction
 endclass
 class Base #(int W=17);
   function int width(); return W; endfunction
 endclass
 class Shadow extends Base;
   function int inherited_width(); return W; endfunction
 endclass
endpackage
module child #(parameter int W=5);
 import cfg::*;
 logic [W-1:0] data;
 initial begin
   if(W!=5 || $bits(data)!=5 || get_w()!=8) $fatal(1,"child scope");
 end
endmodule
module top #(parameter int W=3);
 import cfg::*;
 logic [W-1:0] data;
 child #(.W(5)) u();
 function automatic int local_width(input int W); return W; endfunction
 initial begin
   Reader reader=new;
   Shadow shadow=new;
   int value;
   capture(value);
   if(W!=3 || $bits(data)!=3 || cfg::W!=8) $fatal(1,"module header");
   if(get_w()!=8 || value!=8 || echo(8'hf3)!==8'hf3 || $bits(echo(0))!=8)
     $fatal(1,"package subroutine parameter or width");
   if(reader.width()!=8 || reader.mask()!==8'hff || $bits(reader.mask())!=8)
     $fatal(1,"package class binding");
   if(shadow.inherited_width()!=17 || local_width(19)!=19)
     $fatal(1,"inner declarations");
   #1 $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn explicit_parameter_import_conflicts_with_header_declaration() {
    let result = xezim::simulate(
        r#"
package cfg; parameter int W=8; endpackage
module top #(parameter int W=3);
 import cfg::W;
 initial $display("%0d",W);
endmodule
"#,
        100,
    );
    let error = match result {
        Ok(_) => panic!("explicit import must conflict"),
        Err(e) => e.to_string(),
    };
    assert!(
        error.contains("W") && error.contains("already declared"),
        "{error}"
    );
}
