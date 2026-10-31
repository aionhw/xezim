use xezim::simulate;

fn check(source: &str) {
    let result = simulate(source, 100).unwrap();
    let output = result
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
fn selected_handle_dynamic_array_actual() {
    check(include_str!("../fixtures/selected_handle_array_actual.sv"));
}

#[test]
fn output_handle_starts_null_on_each_call() {
    check(include_str!("../fixtures/output_handle_default.sv"));
}

#[test]
fn suspended_output_handle_starts_null_on_each_call() {
    check(
        &include_str!("../fixtures/output_handle_default.sv").replace("if(fresh)", "#1; if(fresh)"),
    );
}

#[test]
fn function_output_handle_starts_null_on_each_call() {
    check(
        &include_str!("../fixtures/output_handle_default.sv")
            .replace("task sample", "function void sample")
            .replace("endtask", "endfunction"),
    );
}

#[test]
fn selected_handle_fixed_array_actual() {
    check(
        &include_str!("../fixtures/selected_handle_array_actual.sv")
            .replace("data[]", "data[8]")
            .replace("data=new[8];", "")
            .replace("source[]", "source[8]")
            .replace("source.size()", "$size(source)"),
    );
}

#[test]
fn selected_handle_dynamic_array_copyout() {
    check(
        r#"
class item;
 int data[];
 function new(); data=new[2]; data[0]=99; data[1]=88; endfunction
 function void fill(output int a[]); a=new[3]; a[0]=10; a[2]=12; endfunction
 function void modify(ref int a[]); a[1]=11; endfunction
endclass
class driver;
 function void run(item q[$]);
   foreach(q[i]) begin
     q[i].fill(q[i].data);
     q[i].modify(q[i].data);
     if(q[i].data.size()!=3 || q[i].data[0]!=10 || q[i].data[1]!=11 || q[i].data[2]!=12)
       $fatal(1,"array copyout lost");
   end
 endfunction
endclass
module top;
 initial begin
   item q[$]; driver d=new();
   repeat(2) begin item t=new(); q.push_back(t); end
   d.run(q); $display("PASS"); $finish;
 end
endmodule
"#,
    );
}
