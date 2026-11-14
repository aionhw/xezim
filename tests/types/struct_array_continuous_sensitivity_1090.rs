//! §10.3.2, §7.2: a continuous assignment reading an unpacked struct
//! array depends on each member, including nested and selected structs.

fn check(source: &str, expected: &[&str]) {
    let actual: Vec<_> = xezim::simulate(source, 100_000)
        .expect("simulate struct-array sensitivity")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("T|") || s.starts_with("PASS") || s.starts_with("FAIL"))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn nested_member_change_wakes_assignment() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int value; int tag; } S;
typedef struct { S nested; int tail; } N;
function automatic int total(input N a[2]); return a[0].nested.value+a[1].nested.value+a[1].tail; endfunction
module top;
N source[2];
int result;
assign result=total(source);
initial begin
#1 source[0]='{'{0,0},0}; source[1]='{'{0,0},0};
#1 source[1].nested.value=31; source[1].tail=37;
#1 $display("T|nested %0d",result);
$finish;
end
endmodule
"#,
        &["T|nested 68"],
    );
}

#[test]
fn descending_member_change_wakes_assignment() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int value; int tag; } S;
typedef struct { S nested; int tail; } N;
function automatic int total(input S a[3:2]); return a[3].value+a[2].tag; endfunction
module top;
S source[3:2];
int result;
assign result=total(source);
initial begin
#1 source[3]='{0,0}; source[2]='{0,0};
#1 source[3].value=11; source[2].tag=13;
#1 $display("T|descending %0d",result);
$finish;
end
endmodule
"#,
        &["T|descending 24"],
    );
}

#[test]
fn scalar_member_change_wakes_assignment() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int value; int tag; } S;
typedef struct { S nested; int tail; } N;
function automatic int total(input S a); return a.value+a.tag; endfunction
module top;
S source;
int result;
assign result=total(source);
initial begin
#1 source='{0,0};
#1 source.value=23; source.tag=29;
#1 $display("T|scalar %0d",result);
$finish;
end
endmodule
"#,
        &["T|scalar 52"],
    );
}

#[test]
fn element_member_change_wakes_assignment() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int value; int tag; } S;
typedef struct { S nested; int tail; } N;
function automatic int total(input S a); return a.value+a.tag; endfunction
module top;
S source[2];
int result;
assign result=total(source[1]);
initial begin
#1 source[0]='{0,0}; source[1]='{0,0};
#1 source[1].value=41; source[1].tag=43;
#1 $display("T|element %0d",result);
$finish;
end
endmodule
"#,
        &["T|element 84"],
    );
}

#[test]
fn reporter_function_array_argument() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int value; int tag; } S;
function automatic S sum(input S a[2]);
  return '{a[0].value+a[1].value,a[0].tag+a[1].tag};
endfunction
module top;
  S source[2];
  S result;
  assign result=sum(source);
  initial begin
    #1 source[0]='{0,0}; source[1]='{0,0};
    #1 source[1].value=7;
    #1 if(result.value===7) $display("PASS sum=%0d expected=7",result.value);
       else $display("FAIL sum=%0d expected=7",result.value);
    $finish;
  end
endmodule
"#,
        &["PASS sum=7 expected=7"],
    );
}

#[test]
fn instance_and_hierarchical_arguments_keep_their_scope() {
    check(
        r#"
`timescale 1ns/1ps
typedef struct { int value; int tag; } S;
function automatic S sum(input S a[2]);
  return '{a[0].value+a[1].value,a[0].tag+a[1].tag};
endfunction
module unit;
  S source[2], result;
  assign result=sum(source);
endmodule
module top;
  S source[2], result, hierarchical;
  unit u();
  assign result=sum(source);
  assign hierarchical=sum(u.source);
  initial begin
    #1 source[0]='{0,0}; source[1]='{0,0};
       u.source[0]='{0,0}; u.source[1]='{0,0};
    #1 u.source[1].value=7;
    #1 $display("T|child %0d %0d %0d",result.value,u.result.value,hierarchical.value);
    #1 source[0].tag=11;
    #1 $display("T|parent %0d %0d %0d",result.tag,u.result.tag,hierarchical.tag);
    #1 u.source[0].tag=13;
    #1 $display("T|again %0d %0d %0d",result.tag,u.result.tag,hierarchical.tag);
    $finish;
  end
endmodule
"#,
        &["T|child 0 7 7", "T|parent 11 0 0", "T|again 11 13 13"],
    );
}
