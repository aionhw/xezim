//! IEEE 1800-2023 §18.5, §18.5.4, §18.5.7, §18.5.14: simultaneous hard
//! bounds support wide arithmetic independently of source/block order. Guarded
//! and soft constraints must keep their original activation and priority.

fn check(src: &str) {
    let sim = xezim::simulate(src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}

#[test]
fn wide_products_use_hard_bounds_regardless_of_constraint_order() {
    let bounds = "x inside {[1000:1000000]}; y dist {[1000:1000000] := 1};";
    let relation = "z >= x*y; z <= x*y+1; z <= 64'd1000000000000;";
    for constraints in [
        format!("constraint c {{ {relation} {bounds} }}"),
        format!("constraint c {{ {bounds} {relation} }}"),
        format!("constraint a {{ {relation} }} constraint z {{ {bounds} }}"),
        format!("constraint z {{ {relation} }} constraint a {{ {bounds} }}"),
    ] {
        check(&format!(
            r#"
class C;
 rand longint unsigned x,y;
 rand bit [64:0] z;
 {constraints}
endclass
module top;
 initial begin
   C c=new;
   repeat(4) begin
     if(!c.randomize() || c.z<c.x*c.y || c.z>c.x*c.y+1 ||
        c.x<1000 || c.x>1000000 || c.y<1000 || c.y>1000000)
       $fatal(1,"wide product bounds");
   end
   $display("T|PASS");
 end
endmodule
"#
        ));
    }
}

#[test]
fn conditional_disjunctive_and_soft_bounds_are_not_promoted() {
    check(
        r#"
class C;
 rand longint unsigned x,y;
 rand bit [64:0] z;
 rand bit mode;
 constraint guarded {
   if(mode==0) { x inside {[1:2]}; }
   (mode==0) -> y inside {[1:2]};
   (x<10 || x==50);
   soft x==1;
   soft y inside {[1:2]};
 }
 constraint hard {
   z>=x*y;
   z<=x*y+1;
   mode==1;
   x==50;
   y==60;
   solve x before z;
 }
endclass
module top;
 initial begin
   C c=new;
   repeat(4) begin
     if(!c.randomize() || c.x!=50 || c.y!=60 || c.mode!=1 ||
        c.z<3000 || c.z>3001) $fatal(1,"guarded bounds");
   end
   $display("T|PASS");
 end
endmodule
"#,
    );
}
