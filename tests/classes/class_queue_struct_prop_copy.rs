//! §7.6 / §7.10: assigning one class queue property to another, where the
//! element is an unpacked struct with a string member, copies every member
//! of every element along with the size.
#[test]
fn class_queue_of_string_structs_copies_to_another_property() {
    let result = xezim::simulate(
        r#"
typedef struct { string name; int unsigned sz; bit [2:0] xwr; } region_t;
class src_c;
  region_t q[$];
  function new();
    q.push_back('{name:"a", sz: 4, xwr: 3'b111});
    q.push_back('{name:"b", sz: 8, xwr: 3'b101});
  endfunction
endclass
class dst_c;
  src_c cfg;
  region_t s[$];
  function void pull(); s = cfg.q; endfunction
  function void show(string tag);
    $display("%s %0d %s/%0d/%0d %s/%0d/%0d", tag, s.size(),
             s[0].name, s[0].sz, s[0].xwr, s[1].name, s[1].sz, s[1].xwr);
  endfunction
endclass
module t;
  initial begin
    src_c a = new();
    dst_c d = new();
    dst_c e = new();
    d.cfg = a;
    d.pull();
    d.show("method");
    e.s = d.s;
    e.show("handle");
    void'(a.q.pop_back());
    d.pull();
    $display("shrunk %0d %s", d.s.size(), d.s[0].name);
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
    assert_eq!(
        lines,
        ["method 2 a/4/7 b/8/5", "handle 2 a/4/7 b/8/5", "shrunk 1 a"]
    );
}
