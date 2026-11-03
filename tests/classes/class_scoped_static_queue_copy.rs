//! §8.23 / §7.6: `C::q` names a static queue or dynamic array of class `C`,
//! and assigning it as a whole resizes the target and copies every element,
//! into a module variable or into a property of another class.
#[test]
fn class_scoped_static_queue_copies_whole() {
    let result = xezim::simulate(
        r#"
typedef enum { RED, GREEN, BLUE } color_t;
class pool_c;
  static color_t basic[$];
  static int dyn[];
  static function void build();
    basic.push_back(RED);
    basic.push_back(GREEN);
    basic.push_back(BLUE);
    dyn = new[3];
    foreach (dyn[i]) dyn[i] = 10 * (i + 1);
  endfunction
endclass
class user_c;
  color_t allowed[$];
  int d[];
  function void setup();
    allowed = pool_c::basic;
    d = pool_c::dyn;
  endfunction
endclass
module t;
  initial begin
    user_c u = new();
    color_t q[$];
    int d[];
    pool_c::build();
    q = pool_c::basic;
    d = pool_c::dyn;
    $display("module %0d %s %0d %0d", q.size(), q[2].name(), d.size(), d[2]);
    u.setup();
    $display("method %0d %s %0d %0d", u.allowed.size(), u.allowed[1].name(), u.d.size(), u.d[1]);
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
    assert_eq!(lines, ["module 3 BLUE 3 30", "method 3 GREEN 3 20"]);
}
