//! §11.4.13 / §18.5: associative-array inside sets contain values, not keys.
use xezim::simulate;

#[test]
fn inside_associative_values() {
    let src = r#"class choice;
 rand int unsigned selected;
 int allowed[int];
 constraint c { if (allowed.size()) selected inside {allowed}; }
 function new(); allowed[2]=7; allowed[4]=11; endfunction
endclass
module top;
 initial begin
  choice c=new();
  repeat(5) begin
   if(!c.randomize()) $fatal(1,"randomization failed");
   if(!(c.selected inside {7,11})) $fatal(1,"wrong value");
  end
  $display("PASS");$finish;
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
fn inside_string_keys_empty_and_negative_values() {
    let src = r#"
class choice;
 rand int selected;
 int allowed[string];
 constraint c { selected inside {allowed}; }
endclass
module top;
 choice c;
 initial begin
  c=new(); c.selected=123;
  if(c.randomize()) $fatal(1,"empty set accepted");
  if(c.selected!=123) $fatal(1,"failed randomize changed state");
  c.allowed["left"]=-7; c.allowed["right"]=11;
  repeat(10) begin
   if(!c.randomize()) $fatal(1,"nonempty set rejected");
   if(!(c.selected inside {-7,11})) $fatal(1,"selected a key or wrong value");
   if(!(c.selected inside {c.allowed})) $fatal(1,"expression membership failed");
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
