//! A `foreach(s[i])` over a STRING must iterate the string's characters by
//! CONTENT length — and the string arm must be checked BEFORE the associative /
//! dynamic / fixed-array arms.
//!
//! A string name can carry a stale collection registration (e.g. UVM's
//! `pack_string` iterates a string formal whose name is also present as a
//! fixed-array all -*- 2-element registration `(0,-1,1)`). With the string arm
//! ordered last, such a string fell into the fixed-array arm and iterated 2
//! instead of its true character count, truncating a packed bitstream by the
//! missing bytes. These checks pin the corrected precedence and semantics.

use xezim::simulate;

fn sim_src(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const STRING_FOREACH_PRECEDENCE: &str = r#"
module top;
  // The shape UVM's pack_string relies on: a string passed by value whose
  // name also appears as a (stale) array registration must still iterate its
  // characters, not a bogus 2.
  function automatic int packlen(string value);
    int n = 0;
    foreach (value[index]) n++;
    return n;
  endfunction
  initial begin
    string value;
    int n;
    value = "abcdefghijk";          // 11 chars
    n = packlen(value);
    $display("LEN %0d", n);          // must be 11
    // build a string the way uvm unpack does: append then char-write
    string t = "";
    t = {t, " "}; t[0] = "A";
    t = {t, " "}; t[1] = "B";
    t = {t, " "}; t[2] = "C";
    $display("LEN2 %0d %0d", t.len(), packlen(t));   // 3 3
    if (n == 11 && t.len() == 3 && packlen(t) == 3 && t == "ABC")
      $display("TAG_PASS");
    else
      $display("TAG_FAIL");
  end
endmodule
"#;

#[test]
fn string_foreach_uses_content_len_first() {
    let out = sim_src(STRING_FOREACH_PRECEDENCE);
    let msg = out.join("\n");
    assert!(msg.contains("TAG_PASS"), "string foreach precedence broke:\n{msg}");
}