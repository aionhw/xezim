//! §18.5.4/§18.5.6/§18.5.7: conditional and collection constraints in the repair solver.
use xezim::simulate;
fn source() -> &'static str {
    r#"class config_t;
 bit [63:0] lo[]='{64'h100000000};
 bit [63:0] hi[]='{64'h1ffffffff};
 bit [63:0] lo2[]='{64'h1000000000};
 bit [63:0] hi2[]='{64'hffffffffffffffff};
endclass
class transaction;
 config_t cfg=new();
 rand bit [63:0] addr;
 rand int unsigned index,index2;
 typedef enum {NONSHAREABLE,INNERSHAREABLE,OUTERSHAREABLE,SYSTEMSHAREABLE} domain_t;
 rand domain_t domain;
 typedef enum {BAD0,BAD1,GOOD} ref_t;
 rand ref_t r0,r1,r2,r3,r4,r5,r6,r7,r8,r9,r10,r11,r12,r13,r14,r15,r16,r17,r18,r19,r20,r21,r22,r23;
 rand int data[];
 constraint refs { if(cfg.lo.size()>0) {
 !(r0 inside {BAD0,BAD1});
 !(r1 inside {BAD0,BAD1});
 !(r2 inside {BAD0,BAD1});
 !(r3 inside {BAD0,BAD1});
 !(r4 inside {BAD0,BAD1});
 !(r5 inside {BAD0,BAD1});
 !(r6 inside {BAD0,BAD1});
 !(r7 inside {BAD0,BAD1});
 !(r8 inside {BAD0,BAD1});
 !(r9 inside {BAD0,BAD1});
 !(r10 inside {BAD0,BAD1});
 !(r11 inside {BAD0,BAD1});
 !(r12 inside {BAD0,BAD1});
 !(r13 inside {BAD0,BAD1});
 !(r14 inside {BAD0,BAD1});
 !(r15 inside {BAD0,BAD1});
 !(r16 inside {BAD0,BAD1});
 !(r17 inside {BAD0,BAD1});
 !(r18 inside {BAD0,BAD1});
 !(r19 inside {BAD0,BAD1});
 !(r20 inside {BAD0,BAD1});
 !(r21 inside {BAD0,BAD1});
 !(r22 inside {BAD0,BAD1});
 !(r23 inside {BAD0,BAD1});
 } }
 constraint c {
  data.size()==1;
  index < cfg.lo.size(); index2 < cfg.lo2.size();
  if(domain==INNERSHAREABLE)
   foreach(cfg.lo[i]) (index==i) -> addr inside {[cfg.lo[i]:cfg.hi[i]]};
  if(domain==OUTERSHAREABLE)
   foreach(cfg.lo2[i]) (index2==i) -> addr inside {[cfg.lo2[i]:cfg.hi2[i]]};
  foreach(cfg.lo[i]) foreach(cfg.lo2[j])
   (index==i && index2==j) -> addr inside {[cfg.lo[i]:cfg.hi[i]],[cfg.lo2[j]:cfg.hi2[j]]};
  domain inside {INNERSHAREABLE,OUTERSHAREABLE};
 }
endclass
module top;
 initial begin
  transaction t=new();
  repeat(5) begin
   if(!t.randomize() with {addr==64'h100000000;}) $fatal(1,"address failed");
   if(t.r0!=transaction::GOOD || t.r1!=transaction::GOOD || t.r2!=transaction::GOOD || t.r3!=transaction::GOOD || t.r4!=transaction::GOOD || t.r5!=transaction::GOOD || t.r6!=transaction::GOOD || t.r7!=transaction::GOOD || t.r8!=transaction::GOOD || t.r9!=transaction::GOOD || t.r10!=transaction::GOOD || t.r11!=transaction::GOOD || t.r12!=transaction::GOOD || t.r13!=transaction::GOOD || t.r14!=transaction::GOOD || t.r15!=transaction::GOOD || t.r16!=transaction::GOOD || t.r17!=transaction::GOOD || t.r18!=transaction::GOOD || t.r19!=transaction::GOOD || t.r20!=transaction::GOOD || t.r21!=transaction::GOOD || t.r22!=transaction::GOOD || t.r23!=transaction::GOOD) $fatal(1,"excluded enum value");
   if(t.domain!=transaction::INNERSHAREABLE) $fatal(1,"wrong domain");
   if(t.addr!=64'h100000000) $fatal(1,"wrong address");
  end
  $display("PASS"); $finish;
 end
endmodule
"#
}

fn check(src: &str) {
    let sim = simulate(src, 100).expect("simulation");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("PASS"), "{out}");
    assert!(!out.contains("Fatal"), "{out}");
}

#[test]
fn conditional_exclusions_with_nested_address_ranges() {
    check(source());
}

#[test]
fn inactive_exclusions_and_else_branch() {
    let src = source()
        .replace("if(cfg.lo.size()>0)", "if(0)")
        .replace(" } }", " } else { r0 != BAD1; r0 != GOOD; } }")
        .replace(
            "addr==64'h100000000;}",
            "addr==64'h100000000; r0==transaction::BAD0;}",
        )
        .lines()
        .filter(|line| !line.contains("excluded enum value"))
        .collect::<Vec<_>>()
        .join("\n");
    check(&src);
}

#[test]
fn implication_exclusions_with_nested_address_ranges() {
    let src = source().replace("if(cfg.lo.size()>0)", "(cfg.lo.size()>0) ->");
    check(&src);
}

#[test]
fn dynamic_element_dist_after_fallback_draw() {
    let src = source()
        .replace(" rand int data[];", " rand int data[]; function new(); data=new[1]; endfunction")
        .replace("  data.size()==1;", "  foreach(data[k]) data[k] dist {0:=1,[1:4]:/3,[5:16]:/1};")
        .replace("   if(t.addr!=", "   foreach(t.data[k]) if(!(t.data[k] inside {[0:16]})) $fatal(1,\"bad delay\");\n   if(t.addr!=");
    check(&src);
}

#[test]
fn conditional_dynamic_element_equality_after_draw() {
    let src = source()
        .replace("  data.size()==1;", "  if(cfg.lo.size()>0) { data.size()==1; data[0]==1; } else { data.size()==0; }")
        .replace("   if(t.addr!=", "   if(t.data.size()!=1 || t.data[0]!=1) $fatal(1,\"element equality lost\");\n   if(t.addr!=");
    check(&src);
    let src = src
        .replace("if(cfg.lo.size()>0)", "(cfg.lo.size()>0) ->")
        .replace(" else { data.size()==0; }", "");
    check(&src);
}

#[test]
fn conditional_foreach_dist_after_draw() {
    let src = source()
        .replace(" rand int data[];", " rand int data[]; function new(); data=new[1]; endfunction")
        .replace("  data.size()==1;", "  if(cfg.lo.size()>0) { foreach(data[k]) data[k] dist {0:=1,[1:4]:/3,[5:16]:/1}; }")
        .replace("   if(t.addr!=", "   foreach(t.data[k]) if(!(t.data[k] inside {[0:16]})) $fatal(1,\"bad conditional delay\");\n   if(t.addr!=");
    check(&src);
}

#[test]
fn nested_foreach_over_configuration_dynamic_arrays_is_checked() {
    check(
        r#"
class config_t;
 int lo[]='{10};
 int hi[]='{20};
endclass
class T;
 config_t cfg=new();
 rand int value;
 constraint c {
  foreach(cfg.lo[i]) foreach(cfg.hi[j])
   value inside {[cfg.lo[i]:cfg.hi[j]]};
 }
endclass
module top;
 initial begin
  T t=new();
  if(t.randomize() with {value==30;}) $fatal(1,"invalid value accepted");
  if(!t.randomize() with {value==15;}) $fatal(1,"valid value rejected");
  t.cfg.lo=new[0];
  if(!t.randomize() with {value==30;}) $fatal(1,"empty foreach not vacuous");
  $display("PASS"); $finish;
 end
endmodule
"#,
    );
}
