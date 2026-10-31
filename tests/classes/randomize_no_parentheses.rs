//! IEEE 1800 §18.7: omitted empty parentheses must retain inline constraints.
use xezim::simulate;

fn check(source: &str) {
    let sim = simulate(source, 100).unwrap();
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
fn guarded_distribution_uses_caller_weights_without_parentheses() {
    check(include_str!("../fixtures/randomize_no_parentheses.sv"));
}

#[test]
fn bare_inline_randomize_without_parentheses_enforces_and_rolls_back() {
    check(
        r#"
class item;
 rand int x;
 constraint bounds {x inside {[0:10]};}
 function void test();
   if(!randomize with {x==7;}) $fatal(1,"satisfiable call rejected");
   if(x!=7) $fatal(1,"inline constraint lost");
   if(randomize with {x==20;}) $fatal(1,"inconsistent constraints accepted");
   if(x!=7) $fatal(1,"failed solve changed state");
 endfunction
endclass
module top;
 initial begin item obj=new();obj.test();$display("PASS");$finish;end
endmodule
"#,
    );
}

#[test]
fn indexed_receiver_without_parentheses_enforces_inline_constraint() {
    check(
        r#"
class item;rand int x;endclass
module top;
 initial begin item objects[1];objects[0]=new();
 if(!objects[0].randomize with {x==19;})$fatal(1,"solve failed");
 if(objects[0].x!=19)$fatal(1,"indexed receiver constraint lost");
 $display("PASS");$finish;end
endmodule
"#,
    );
}
