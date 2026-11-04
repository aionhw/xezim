//! IEEE 1800-2017 §7.10.1, §11.4.12: `$` in a queue subscript is
//! relative to that queue, also when selecting a property of its object.
use xezim::simulate;

#[test]
fn queue_last_object_member_through_formal() {
    let sim = simulate(
        r#"class field_info;
  int flags;
endclass
class field_list;
  field_info contents[$];
endclass
class builder;
  function void add(field_list list, int flags);
    field_info f = new();
    list.contents.push_back(f);
    list.contents[$].flags = flags;
  endfunction
endclass
module top;
  initial begin
    builder b = new();
    field_list list = new();
    b.add(list, 7);
    b.add(list, 13);
    $display("flags %0d %0d size %0d", list.contents[0].flags,
             list.contents[1].flags, list.contents.size());
    $finish;
  end
endmodule
"#,
        10,
    )
    .expect("simulate");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(out, ["flags 7 13 size 2"]);
}

#[test]
fn queue_member_bounds_follow_each_object_and_relative_select() {
    let sim = simulate(
        r#"class item;
  int flags;
endclass
class holder;
  item q[$];
  function void add();
    item it = new();
    q.push_back(it);
  endfunction
  function void set_flags();
    q[$-1].flags = 21;
    this.q[$].flags = 32;
    this.q[$].flags[3:0] = 5;
  endfunction
  task delayed_flags();
    q[$].flags <= 77;
    add();
    #1;
  endtask
endclass
module top;
  holder a, b;
  initial begin
    a = new(); b = new();
    a.add(); a.add(); b.add();
    a.set_flags();
    b.q[$].flags = 43;
    a.q[$-1].flags = 54;
    $display("members %0d %0d %0d", a.q[0].flags, a.q[1].flags, b.q[0].flags);
    a.delayed_flags();
    $display("nba %0d %0d size %0d", a.q[1].flags, a.q[2].flags, a.q.size());
    $finish;
  end
endmodule
"#,
        10,
    )
    .expect("simulate");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(out, ["members 54 37 43", "nba 77 0 size 3"]);
}
