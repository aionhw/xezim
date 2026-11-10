//! §8.23 / §10.10: inside a class body, `C::aq[k]` (an element of a static
//! associative array of queues of class `C`) is a whole queue, so as an item
//! of an unpacked-array concatenation it adds every element of that queue.
#[test]
fn class_scoped_assoc_queue_element_in_concatenation() {
    let result = xezim::simulate(
        r#"
typedef enum { RED, GREEN, BLUE, CYAN, GRAY } color_t;
typedef enum { WARM, COOL } group_t;
class pool_c;
  static color_t by_group[group_t][$];
  static function void build();
    by_group[WARM].push_back(RED);
    by_group[WARM].push_back(GREEN);
    by_group[COOL].push_back(BLUE);
    by_group[COOL].push_back(CYAN);
  endfunction
endclass
class user_c;
  color_t allowed[$];
  function void setup();
    allowed.push_back(GRAY);
    allowed = {allowed, pool_c::by_group[COOL], pool_c::by_group[WARM]};
  endfunction
endclass
module t;
  initial begin
    user_c u = new();
    pool_c::build();
    u.setup();
    $display("%0d %s %s %s", u.allowed.size(), u.allowed[1].name(), u.allowed[2].name(),
             u.allowed[4].name());
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["5 BLUE CYAN GREEN"]);
}

/// Each index of `C::q[s][i]` takes the key type of the dimension it selects
/// into, so a string-then-int row names the queue a `push_back` filled.
#[test]
fn class_scoped_assoc_queue_element_mixed_key_types() {
    let result = xezim::simulate(
        r#"
class pool_c;
  static int rows[string][int][$];
  static function void build();
    rows["x"][65].push_back(9);
    rows["x"][65].push_back(8);
    rows["x"][66].push_back(7);
  endfunction
endclass
class user_c;
  int got[$];
  function void setup();
    got = {pool_c::rows["x"][65], pool_c::rows["x"][66]};
  endfunction
endclass
module t;
  initial begin
    user_c u = new();
    pool_c::build();
    u.setup();
    $display("%0d %0d %0d %0d", u.got.size(), u.got[0], u.got[1], u.got[2]);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["3 9 8 7"]);
}
