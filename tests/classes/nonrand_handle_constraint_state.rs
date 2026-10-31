//! IEEE 1800 §18.4/§18.5.9: a non-rand handle's fields are state.
use xezim::simulate;

#[test]
fn nonrand_config_remains_state() {
    let src = r#"class config_t;
 rand int cache_line_size=64;
endclass
class transaction;
 config_t cfg;
 rand int burst_length;
 rand int burst_size;
 constraint c {
  burst_length inside {1,2,4,8,16};
  burst_size inside {[0:3]};
  (burst_length << burst_size)==cfg.cache_line_size;
 }
 function new(config_t c); cfg=c; endfunction
endclass
module top;
 initial begin
  config_t c=new();
  transaction t=new(c);
  repeat(10) begin
   if(!t.randomize()) $fatal(1,"randomization failed");
   if(c.cache_line_size!=64) $fatal(1,"modified state config: %0d",c.cache_line_size);
  end
  $display("PASS"); $finish;
 end
endmodule
"#;
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
fn disabled_handle_and_child_fields_remain_state() {
    let src = r#"
class config_t;
 rand int value=64;
endclass
class transaction;
 rand config_t cfg;
 rand int value;
 constraint c { value==7; cfg.value==value; }
 function new(); cfg=new(); endfunction
endclass
module top;
 initial begin
  transaction t=new();
  t.cfg.rand_mode(0);
  if(t.randomize()) $fatal(1,"disabled child accepted conflicting constraint");
  if(t.cfg.value!=64) $fatal(1,"disabled child changed");
  t.cfg.rand_mode(1);
  if(!t.randomize()) $fatal(1,"enabled rand child rejected");
  if(t.cfg.value!=7 || t.value!=7) $fatal(1,"cross-object equality failed");
  $display("PASS"); $finish;
 end
endmodule
"#;
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
