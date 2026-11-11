//! IEEE 1800-2023 §18.5, §11.4.12 and §11.6: concatenation equality
//! preserves unsigned self-determined fields and solves aligned sparse relations.

fn check(src: &str) {
    let sim = xezim::simulate(src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}

#[test]
fn aligned_fields_solve_across_search_seeds() {
    check(
        r#"
class Pair;
 rand bit [15:0] a,b,c,d;
 constraint same { {a,b}=={c,d}; }
endclass
module top;
 initial begin
 Pair p=new;
 for(int seed=0;seed<32;seed++) begin
   p.srandom(seed);
   if(!p.randomize() || {p.a,p.b}!=={p.c,p.d}) $fatal(1,"concat seed=%0d",seed);
 end
 $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn nested_signed_fields_and_arithmetic_keep_their_own_widths() {
    check(
        r#"
class Nested;
 rand byte signed a,b;
 rand bit [7:0] c,d;
 constraint same { {{a},b}=={c,{d}}; a==-1; b==-128; }
endclass
class Wrapped;
 rand bit [7:0] a,b,c,d;
 constraint same { {a+8'd1,b}=={c,d}; a==255; b==10; }
endclass
module top;
 initial begin
 Nested n=new; Wrapped w=new;
 if(!n.randomize() || n.c!==8'hff || n.d!==8'h80) $fatal(1,"signed fields");
 if(!w.randomize() || w.c!==0 || w.d!==10) $fatal(1,"field arithmetic overflow");
 $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn unaligned_width_extension_and_negated_relations_keep_bit_semantics() {
    check(
        r#"
class Unaligned;
 rand bit [7:0] a,b;
 rand bit [11:0] c;
 rand bit [3:0] d;
 constraint same { {a,b}=={c,d}; a==8'h12; b==8'h34; }
endclass
class Extended;
 rand bit [3:0] a,b,d;
 rand bit [7:0] c;
 constraint same { {a,b}=={c,d}; a==4'ha; b==4'hb; }
endclass
class Different;
 rand bit [7:0] a,b,c,d;
 constraint different { !({a,b}=={c,d}); a==1; b==2; c==1; d==3; }
endclass
class Impossible;
 rand byte signed a,b;
 rand bit [7:0] c,d;
 constraint same { {a,b}=={c,d}; a==-1; c==0; }
endclass
module top;
 initial begin
 Unaligned u=new; Extended e=new; Different d=new; Impossible i=new;
 if(!u.randomize() || u.c!==12'h123 || u.d!==4'h4) $fatal(1,"unaligned fields");
 if(!e.randomize() || e.c!==8'ha || e.d!==4'hb) $fatal(1,"zero extension");
 if(!d.randomize() || {d.a,d.b}=={d.c,d.d}) $fatal(1,"negated equality");
 if(i.randomize()) $fatal(1,"unsatisfiable signed field equality");
 $display("T|PASS");
 end
endmodule
"#,
    );
}
