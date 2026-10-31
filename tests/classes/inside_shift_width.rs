//! IEEE 1800 §11.4.10/§11.4.13: oversized shifts and joint inside types.
use xezim::simulate;

fn check(source: &str) {
    let sim = simulate(source, 1000).unwrap();
    let output = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.contains("PASS") && !output.contains("Fatal"),
        "{output}"
    );
}
#[test]
fn inside_shift_to_full_unsigned_address_range() {
    check(include_str!("../fixtures/inside_shift_width.sv"));
}
#[test]
fn oversized_shifts_keep_logical_zero_and_arithmetic_sign_fill() {
    check(
        r#"
class item;
 rand longint signed a;
 rand longint unsigned b;
 constraint c { a == (-64'sd1 >>> 64); b == (64'hffff_ffff_ffff_ffff >> 64); }
endclass
module top; initial begin item x=new();
 if(!x.randomize())$fatal(1,"shift solve failed");
 if(x.a!=-1 || x.b!=0)$fatal(1,"shift fill wrong");
 $display("PASS");$finish;end endmodule
"#,
    );
}
#[test]
fn scope_randomize_uses_unsigned_inside_context() {
    check(
        r#"
module top;bit[63:0] x;int width=64;initial begin
 if(!std::randomize(x) with {x inside {[0:((1<<width)-1)]}; x==64'hfedcba9876543210;})$fatal(1,"scope solve failed");
 if(x!=64'hfedcba9876543210)$fatal(1,"scope value wrong");
 $display("PASS");$finish;end endmodule
"#,
    );
}

#[test]
fn procedural_inside_sizes_64_and_128_bit_integer_bounds_and_preserves_reals() {
    check(include_str!("../fixtures/inside_procedural_width.sv"));
}

#[test]
fn comparison_bound_preserves_shift_until_context_is_known() {
    check(include_str!("../fixtures/compare_shift_bound.sv"));
}

#[test]
fn shifted_comparison_bounds_at_32_64_and_128_bits() {
    check(r#"
class bound32; rand bit[31:0] a; rand int b; constraint c {a == (1<<48); b == (1<<31);} endclass
class bound64; rand bit[63:0] a; constraint c {a < (1<<63);} endclass
class wide; rand bit[127:0] a; constraint c {a < (1<<96); a[95:80]!=0;} endclass
module top;initial begin
 bound32 s=new();bound64 l=new();wide w=new();
 if(!s.randomize() || s.a!=0 || s.b!=32'sh80000000)$fatal(1,"32-bit overshift");
 if(!l.randomize() || l.a[63])$fatal(1,"64-bit bound");
 w.a=128'b1<<80;
 if(!w.randomize(null))$fatal(1,"known valid 128-bit bound");
 if(!w.randomize() || w.a[127:96]!=0 || w.a[95:80]==0)$fatal(1,"128-bit bound");
 $display("PASS");$finish;
 end endmodule
"#);
}

#[test]
fn packed_slice_inequality_is_not_array_uniqueness() {
    check(r#"
class transaction;
 rand bit[31:0] a;
 constraint c {a[23:8]!=0; a[23:8]!=16'hffff;}
endclass
module top;initial begin transaction t=new();
 repeat(10) begin
 if(!t.randomize())$fatal(1,"packed slice solve");
 if(t.a[23:8]==0 || t.a[23:8]==16'hffff)$fatal(1,"packed slice value");
 end
 $display("PASS");$finish;end endmodule
"#);
}
