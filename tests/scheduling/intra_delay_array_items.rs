//! IEEE 1800-2023 §9.4.5, §10.9.1, §10.10: an assignment pattern with an
//! intra-assignment delay or event control reads its items when the
//! statement runs. An item that is an unpacked array (`wb = #1 '{wc, r}`)
//! supplies the elements of the sub-array it is matched to. The capture
//! kept such an item as one scalar, so the delayed store wrote zeros.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn delayed_whole_array_items_positional_keyed_and_default() {
    let msgs = run(r##"
module top;
  typedef int row_t [2];
  int wc [2] = '{1, 2};
  row_t r = '{5, 6};
  int wb [2][2];
  int wk [2][2];
  int wf [2][2];
  int dd [1:0][2];
  initial begin
    wb = #1 '{wc, r};
    wk = #1 '{1: wc, 0: r};
    wf = #1 '{default: wc};
    dd = #1 '{wc, r};
    $display("wb=%p wk=%p wf=%p dd1=%p dd0=%p", wb, wk, wf, dd[1], dd[0]);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "wb='{'{1, 2}, '{5, 6}} wk='{'{5, 6}, '{1, 2}} wf='{'{1, 2}, '{1, 2}} dd1='{1, 2} dd0='{5, 6}"
        ]
    );
}

#[test]
fn delayed_rows_and_three_dimensional_items() {
    let msgs = run(r##"
module top;
  int wc [2] = '{1, 2};
  int m [2][2] = '{'{3, 4}, '{5, 6}};
  int w3 [2][2][2];
  int wr [2][2];
  initial begin
    w3 = #1 '{m, '{m[1], wc}};
    wr = #1 '{m[1], m[0]};
    $display("w3=%p wr=%p", w3, wr);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["w3='{'{'{3, 4}, '{5, 6}}, '{'{5, 6}, '{1, 2}}} wr='{'{5, 6}, '{3, 4}}"]
    );
}

#[test]
fn array_items_are_read_before_the_delay_or_event() {
    let msgs = run(r##"
module top;
  int wc [2] = '{1, 2};
  int m [2][2] = '{'{3, 4}, '{5, 6}};
  int we [2][2];
  int wx [2][2];
  event ev;
  initial we = @(ev) '{wc, m[0]};
  initial wx = #2 '{wc, m[1]};
  initial begin
    #1 wc = '{8, 9};
    m = '{'{0, 0}, '{0, 0}};
    -> ev;
    #2 $display("we=%p wx=%p", we, wx);
  end
endmodule
"##);
    assert_eq!(msgs, ["we='{'{1, 2}, '{3, 4}} wx='{'{1, 2}, '{5, 6}}"]);
}

#[test]
fn nonblocking_and_undelayed_array_items_are_unchanged() {
    let msgs = run(r##"
module top;
  int wc [2] = '{1, 2};
  int v = 7;
  int wn [2][2];
  int wu [2][2];
  int ws [2][2];
  initial begin
    wn <= #1 '{wc, wc};
    wu = '{wc, '{3, 4}};
    ws = #1 '{'{default: v}, '{v, 1}};
    #1 $display("wn=%p wu=%p ws=%p", wn, wu, ws);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        ["wn='{'{1, 2}, '{1, 2}} wu='{'{1, 2}, '{3, 4}} ws='{'{7, 7}, '{7, 1}}"]
    );
}
