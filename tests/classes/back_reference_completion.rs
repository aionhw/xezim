//! §18.5.8.1 / §18.6.2: back-reference constraints honor associative
//! state keys and invoke each child's callback once, after the joint solve.
fn check(src: &str) {
    let sim = xezim::simulate(src, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}

#[test]
fn sibling_callbacks_run_once_on_final_values() {
    check(
        r#"
typedef class txn;
class sibling;
  rand bit [7:0] x;
  int posts;
  int seen;
  function void post_randomize(); posts++; seen=x; endfunction
endclass
class policy;
  txn item;
  constraint c { item.other.x == 7; }
endclass
class txn;
  rand sibling other;
  rand policy p;
  int posts;
  function new(); other=new; p=new; endfunction
  function void pre_randomize(); p.item=this; endfunction
  function void post_randomize();
    posts++;
    if(other.posts != posts || other.seen != 7) $fatal(1,"callback order");
  endfunction
endclass
module top;
 initial begin
  txn t=new;
  t.srandom(1);
  for(int i=1;i<=5;i++) begin
    if(!t.randomize()) $fatal(1,"solve failed");
    if(t.other.posts != i || t.posts != i) $fatal(1,"duplicate callback");
    if(t.other.x != 7 || t.other.seen != 7) $fatal(1,"stale callback value");
  end
  $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn associative_state_constraints_check_string_and_numeric_keys() {
    check(
        r#"
typedef class txn;
class policy;
  txn item;
  int limits[string];
  int upper[int];
  function new(); limits["a"]=200; limits["b"]=210; upper[-2]=240; endfunction
  constraint c {
    foreach(limits[k]) item.addr >= limits[k];
    foreach(upper[k]) item.addr <= upper[k];
  }
endclass
class txn;
  rand bit [7:0] addr;
  rand policy p;
  function new(); p=new; endfunction
  function void pre_randomize(); p.item=this; endfunction
endclass
module top;
 initial begin
  txn t=new;
  t.srandom(1);
  repeat(10) begin
    if(!t.randomize()) $fatal(1,"solve failed");
    if(t.addr < 210 || t.addr > 240) $fatal(1,"constraint ignored");
  end
  t.p.limits.delete(); t.p.upper.delete();
  if(!t.randomize()) $fatal(1,"empty associative array");
  $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn failed_enclosing_solve_does_not_run_child_callbacks() {
    check(
        r#"
typedef class txn;
class child;
  rand bit [7:0] x;
  int posts;
  function void post_randomize(); posts++; endfunction
endclass
class policy;
  txn item;
  constraint c { item.other.x == 7; item.other.x == 8; }
endclass
class txn;
  rand child other;
  rand policy p;
  function new(); other=new; p=new; other.x=3; endfunction
  function void pre_randomize(); p.item=this; endfunction
endclass
module top;
 initial begin
  txn t=new;
  if(t.randomize()) $fatal(1,"impossible solve accepted");
  if(t.other.posts != 0 || t.other.x != 3) $fatal(1,"failed solve changed child");
  $display("T|PASS");
 end
endmodule
"#,
    );
}
