//! IEEE 1800-2017 §7.8.6 / §11.5.1 / §13.4: an associative array written
//! through a select and returned by value (#582 and #583, reported by
//! Ganesh-TNL). A bit, part or indexed part-select write to an ABSENT
//! element was dropped (the element is read as its type's default, the
//! bits replaced and the element created), and an associative array
//! returned from a class method (instance, virtual, static, or through the
//! method's own name) came back empty. Siblings: `f()[k]` and
//! `o.method().size()` on a returned collection, and `%p` of a class
//! member associative array of vectors.

use std::path::Path;
use std::process::Command;
use xezim::simulate;

fn run(file: &str, top: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg(top)
        .arg("--no-cache")
        .arg(path.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn reported_select_write_testbench() {
    let text = run(
        "tests/collections/assoc_select_write_582.sv",
        "test_assoc_partselect",
    );
    assert!(!text.contains("FAIL"), "{text}");
    assert!(text.contains("TEST_PASS"), "{text}");
}

#[test]
fn reported_method_return_testbench() {
    let text = run(
        "tests/collections/assoc_method_return_583.sv",
        "test_class_method_assoc",
    );
    assert!(!text.contains("FAIL"), "{text}");
    assert!(text.contains("TEST_PASS"), "{text}");
}

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

#[test]
fn select_writes_and_returned_collections() {
    let msgs = messages(
        "typedef bit [31:0] word_map_t [int];
typedef int q_t [$];
class B;
  bit [15:0] mem [int];
  function word_map_t build(); word_map_t w; w[200] = 32'hcafef00d; return w; endfunction
  static function word_map_t sbuild(); word_map_t w; w[3] = 3; return w; endfunction
  function word_map_t direct(); direct[9] = 9; direct[2][4] = 1'b1; endfunction
  function q_t mkq(); q_t q; q.push_back(3); return q; endfunction
  function void poke(); mem[3][7:0] = 8'h12; mem[4][15] = 1'b1; endfunction
endclass
module t;
  bit [31:0] m [int];
  logic [31:0] ml [int];
  bit [7:0] sb [string];
  int aq [int][$];
  int aa [int][int];
  function automatic q_t mq(); q_t q; q.push_back(4); q.push_back(5); return q; endfunction
  initial begin
    B o = new; word_map_t w;
    m[100][20:0] = 21'h123456; m[101][3] = 1'b1; m[102][8 +: 8] = 8'hab;
    ml[5][7:0] = 8'h12;
    sb[\"k\"][2] = 1'b1;
    aq[5].push_back(2); aq[5][1] = 7;
    aa[1][2] = 3;
    $display(\"S|%0d %h %h %h %h %h %0d %0d\", m.size(), m[100], m[101], m[102], ml[5], sb[\"k\"], aq[5][1], aa[1][2]);
    w = o.build();  $display(\"R|method %p\", w);
    w = B::sbuild(); $display(\"R|static %p\", w);
    w = o.direct(); $display(\"R|named %p\", w);
    $display(\"C|%0d %h %0d %0d %0d\", o.build().size(), o.build()[200], mq()[1], o.mkq().size(), mq().size());
    o.poke(); o.mem[5][3:0] = 4'h9;
    $display(\"P|%p\", o.mem);
  end
endmodule
",
    );
    assert_line(&msgs, "S|3 00123456 00000008 0000ab00 xxxxxx12 04 7 3");
    assert_line(&msgs, "R|method '{200:'hcafef00d}");
    assert_line(&msgs, "R|static '{3:'h3}");
    assert_line(&msgs, "R|named '{2:'h10, 9:'h9}");
    assert_line(&msgs, "C|1 cafef00d 5 1 2");
    assert_line(&msgs, "P|'{3:'h12, 4:'h8000, 5:'h9}");
}
