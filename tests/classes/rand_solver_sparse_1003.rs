//! IEEE 1800-2023 §18.5, §11.4.3, §11.4.12: satisfiable sparse relations
//! propagate through arithmetic and packed operands rather than random guesses.

fn check(src: &str) {
    let sim = xezim::simulate(src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}

#[test]
fn unsigned_constant_division_propagates_a_quotient_interval() {
    check(
        r#"
class C;
 rand bit [30:0] chunk;
 constraint c { chunk/4096==123456; }
endclass
module top;
 initial begin
   C c=new;
   if(!c.randomize() || c.chunk/4096!=123456) $fatal(1,"division");
   $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn packed_concatenations_constrain_every_operand() {
    check(
        r#"
class C;
 rand bit [7:0] f0,f1,f2,f3;
 constraint c { {f3,f2,f1,f0}==32'hdead_beef; }
endclass
class Pair;
 rand bit [15:0] a,b,c,d;
 constraint same { {a,b}=={c,d}; }
endclass
module top;
 initial begin
   C c=new;
   Pair p=new;
   if(!c.randomize() || {c.f3,c.f2,c.f1,c.f0}!==32'hdead_beef) $fatal(1,"concat constant");
   if(!p.randomize() || {p.a,p.b}!=={p.c,p.d}) $fatal(1,"concat relation");
   $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn squared_rand_variables_propagate_through_a_product() {
    check(
        r#"
class C;
 rand int unsigned x;
 constraint c { x inside {[0:100000]}; x**2==3481; }
endclass
module top;
 initial begin
   C c=new;
   if(!c.randomize() || c.x!=59) $fatal(1,"power");
   $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn scope_randomization_splits_wide_array_elements() {
    check(
        r#"
module top;
 bit [64:0] data[8];
 initial begin
   if(!std::randomize(data) with { foreach(data[i]) data[i][7:0]==8'h42; })
     $fatal(1,"wide scope array");
   foreach(data[i]) if(data[i][7:0]!==8'h42) $fatal(1,"wide scope slice");
   $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn unsigned_remainders_constrain_array_elements() {
    check(
        r#"
class C;
 rand int unsigned a[];
 constraint c { a.size()==4; foreach(a[i]) a[i] inside {[0:2147483647]}; a[0]%65536==12345; }
endclass
class D;
 rand int unsigned x;
 constraint c { x inside {[0:100000]}; (x/7)%13==5; }
endclass
module top;
 initial begin
 C c=new; D d=new;
 if(!c.randomize() || c.a[0]%65536!=12345) $fatal(1,"array remainder");
 if(!d.randomize() || (d.x/7)%13!=5) $fatal(1,"nested quotient");
 $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn wide_array_reductions_propagate_carries_in_class_and_scope() {
    check(
        r#"
class C;
 rand bit [64:0] a[256];
 constraint c { foreach(a[i]) a[i]<=65'hff; a.sum()==65'h8000; }
endclass
module top;
 bit [64:0] data[256];
 initial begin
 C c=new;
 if(!c.randomize() || c.a.sum()!=65'h8000) $fatal(1,"wide class reduction");
 if(!std::randomize(data) with { foreach(data[i]) data[i]<=65'hff; data.sum()==65'h8000; })
   $fatal(1,"wide scope reduction");
 if(data.sum()!=65'h8000) $fatal(1,"wide scope sum");
 $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn sparse_arithmetic_preserves_boundaries_signs_and_unsatisfiable_results() {
    check(
        r#"
class Bound;
 rand int unsigned x;
 constraint c { x inside {[0:100]}; 5==x/13; }
endclass
class Impossible;
 rand int unsigned x;
 constraint c { x<65; x/13==5; }
endclass
class Negative;
 rand int x;
 constraint c { x inside {[-60:-58]}; x**2==3481; }
endclass
class Bits;
 rand byte signed a;
 rand byte signed b;
 constraint c { {a,b}==16'hfe81; }
endclass
class Carry;
 rand bit [64:0] a[2];
 constraint c { a[0]==65'h1ffffffffffffffff; a[1]==2; a.sum()==1; }
endclass
module top;
 initial begin
 Bound b=new; Impossible i=new; Negative n=new; Bits v=new; Carry c=new;
 if(!b.randomize() || b.x<65 || b.x>77) $fatal(1,"quotient interval");
 if(i.randomize()) $fatal(1,"unsatisfiable quotient");
 if(!n.randomize() || n.x!=-59) $fatal(1,"negative square root");
 if(!v.randomize() || {v.a,v.b}!==16'hfe81) $fatal(1,"signed concat");
 if(!c.randomize() || c.a.sum()!=1) $fatal(1,"wide carry wrap");
 $display("T|PASS");
 end
endmodule
"#,
    );
}

/// The original report also couples packed struct fields, dist weights,
/// solve-before order and indexed records, in 64-bit and 65-bit variants.
#[test]
fn consolidated_report_solves_every_satisfiable_case() {
    let sim = xezim::simulate(include_str!("rand_solver_sparse_1003.sv"), 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "TEST_PASS"),
        "{:?}",
        sim.output
    );
    assert!(
        !sim.output.iter().any(|o| o.message.starts_with("FAIL")),
        "{:?}",
        sim.output
    );
}

#[test]
fn indexed_signed_inputs_keep_unsigned_comparison_context() {
    check(
        r#"
class C;
 byte signed data[2]='{-1,-2};
 rand bit idx;
 rand bit [15:0] value;
 constraint c { value==data[idx]; }
endclass
module top;
 initial begin
 C c=new;
 for(int i=0;i<2;i++) begin
 if(!c.randomize() with { idx==local::i; }) $fatal(1,"indexed input");
 if(c.value !== (i==0?16'hff:16'hfe)) $fatal(1,"unsigned input extension");
 end
 $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn wide_arithmetic_carries_borrows_and_bounds_are_exact() {
    check(
        r#"
class Add;
 rand bit [64:0] a,b;
 constraint c { a==65'h0ffffffffffffffff; b==1; a+b==65'h10000000000000000; }
endclass
class Sub;
 rand bit [64:0] a,b;
 constraint c { a==0; b==1; a-b==65'h1ffffffffffffffff; }
endclass
class Multi;
 rand bit [128:0] a,b;
 constraint c { a==129'h0ffffffffffffffffffffffffffffffff; b==1;
   a+b==129'h100000000000000000000000000000000; }
endclass
class Offset;
 rand bit [79:0] base;
 rand int unsigned count, stride;
 constraint c { base==80'h10000000000000000; count inside {[0:1000]}; stride==17;
 base+count*stride==80'h100000000000006a4; }
endclass
class Impossible;
 rand bit [64:0] a,b;
 constraint c { a<10; b<10; a+b==65'h10000000000000000; }
endclass
module top;
 initial begin
 Add a=new; Sub b=new; Multi m=new; Offset o=new; Impossible u=new;
 if(!a.randomize() || a.a+a.b!==65'h10000000000000000) $fatal(1,"carry");
 if(!b.randomize() || b.a-b.b!==65'h1ffffffffffffffff) $fatal(1,"borrow");
 if(!m.randomize() || m.a+m.b!==129'h100000000000000000000000000000000) $fatal(1,"multiple carries");
 if(!o.randomize() || o.count!=100) $fatal(1,"wide offset product");
 if(u.randomize()) $fatal(1,"impossible wide sum");
 $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn record_fields_do_not_alias_same_named_wide_rand_properties() {
    check(
        r#"
typedef struct packed { longint unsigned addr; } record_t;
class C;
 record_t records[$];
 rand bit [64:0] addr;
 rand int index;
 rand longint unsigned base;
 constraint c { index>=0; index<records.size(); base==records[index].addr;
   addr=={1'b1,base}; }
endclass
module top;
 initial begin
 C c=new;
 record_t r;
 r.addr=64'h123456789abcdef0; c.records.push_back(r);
 r.addr=64'hfedcba9876543210; c.records.push_back(r);
 if(!c.randomize() || c.base!=c.records[c.index].addr || c.addr!=={1'b1,c.base})
   $fatal(1,"record field dependency");
 $display("T|PASS");
 end
endmodule
"#,
    );
}
