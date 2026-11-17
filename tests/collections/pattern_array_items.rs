//! §10.9.1, §10.10: an assignment pattern item whose value is itself an
//! unpacked array (a variable, a row, a function result) supplies the
//! elements of the sub-array it is matched to: positionally, by index key
//! or through `default:`. The item was evaluated as one scalar and written
//! to every element below, so `wb = '{wc, wc}` stored zeros.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn whole_array_items_positional_keyed_and_descending() {
    let msgs = run(r##"
module top;
  typedef int row_t [2];
  int wc [2] = '{1, 2};
  row_t r = '{5, 6};
  int wb [2][2];
  int wd [2][2];
  int wk [2][2];
  int dd [1:0][2];
  initial begin
    wb = '{wc, r};
    wd = '{wc, '{7, 8}};
    wk = '{1: wc, 0: r};
    dd = '{wc, r};
    $display("wb=%p wd=%p wk=%p", wb, wd, wk);
    $display("dd1=%p dd0=%p", dd[1], dd[0]);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "wb='{'{1, 2}, '{5, 6}} wd='{'{1, 2}, '{7, 8}} wk='{'{5, 6}, '{1, 2}}",
            "dd1='{1, 2} dd0='{5, 6}",
        ]
    );
}

#[test]
fn default_function_result_and_rows_of_a_three_dimensional_array() {
    let msgs = run(r##"
module top;
  typedef int row_t [2];
  int wc [2] = '{1, 2};
  int m [2][2] = '{'{3, 4}, '{5, 6}};
  int wf [2][2];
  int wx [2][2];
  int w3 [2][2][2];
  int w4 [2][2][2];
  function automatic row_t mk(int x);
    mk = '{x, x + 1};
  endfunction
  initial begin
    wf = '{default: wc};
    wx = '{mk(10), mk(20)};
    w3 = '{m, '{m[1], wc}};
    w4 = '{default: wc};
    $display("wf=%p wx=%p", wf, wx);
    $display("w3=%p", w3);
    $display("w4=%p", w4);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "wf='{'{1, 2}, '{1, 2}} wx='{'{10, 11}, '{20, 21}}",
            "w3='{'{'{3, 4}, '{5, 6}}, '{'{5, 6}, '{1, 2}}}",
            "w4='{'{'{1, 2}, '{1, 2}}, '{'{1, 2}, '{1, 2}}}",
        ]
    );
}

#[test]
fn declaration_initializer_and_struct_array_member() {
    let msgs = run(r##"
module top;
  typedef struct { int a [2]; int b; } s_t;
  int wc [2] = '{1, 2};
  int wi [2][2] = '{wc, '{3, 4}};
  s_t s;
  initial begin
    s = '{wc, 9};
    $display("wi=%p s=%p", wi, s);
  end
endmodule
"##);
    assert_eq!(msgs, ["wi='{'{1, 2}, '{3, 4}} s='{a:'{1, 2}, b:9}"]);
}

#[test]
fn nonblocking_pattern_of_whole_array_items() {
    let msgs = run(r##"
module top;
  int wc [2] = '{1, 2};
  int wn [2][2];
  initial begin
    wn <= '{wc, wc};
    #0 $display("#0: wn=%p", wn);
    #1 $display("#1: wn=%p", wn);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["#0: wn='{'{0, 0}, '{0, 0}}", "#1: wn='{'{1, 2}, '{1, 2}}"]
    );
}

#[test]
fn scalar_items_and_nested_patterns_still_fill_elements() {
    let msgs = run(r##"
module top;
  int v = 7;
  int a [2][2];
  int b [2][2];
  int c [2][2];
  initial begin
    a = '{default: 5};
    b = '{'{1, 2}, '{3, 4}};
    c = '{'{default: v}, '{v, 1}};
    $display("a=%p b=%p c=%p", a, b, c);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["a='{'{5, 5}, '{5, 5}} b='{'{1, 2}, '{3, 4}} c='{'{7, 7}, '{7, 1}}"]
    );
}
