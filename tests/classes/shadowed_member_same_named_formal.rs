//! IEEE 1800-2023 §8.14 / §8.18: `h.p` where `p` is redeclared in a derived class resolves by the STATIC type of `h`, also while another process is suspended in a subroutine whose formal has the same name as `h` and a different class type (a UVM driver's `item` while a sequence waits in `finish_item(uvm_sequence_item item)`).
//!
//! Expected lines come from the reference simulator.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

// The base copy is `local` (§8.18), as `uvm_transaction::accept_time` is.
const LOCAL_BASE: &str = r#"
class B;
  local int p = -1;
  function int base_p(); return p; endfunction
endclass
class D extends B; int p; endclass
class Seq; task finish(B item); #10; endtask endclass
class Drv;
  task drive(D item); #5; item.p = 42; endtask
endclass
module t;
  initial begin
    Drv d = new; Seq s = new; D a = new;
    fork d.drive(a); #1 s.finish(a); join
    $display("T|a|p=%0d base_p=%0d", a.p, a.base_p());
  end
endmodule
"#;

const FAMILY: &str = r#"
class B; int p = -1; function int bp(); return p; endfunction endclass
class D extends B; int p = -2; endclass
class W; D item; endclass
class Blk;
  task hold_b(B item); #10; endtask
  task hold_d(D item); #10; endtask
endclass
class T;
  D item;
  task wr_formal(D item); #5; item.p = 11; endtask
  task rd_formal(D item, output int v); #5; v = item.p; endtask
  task wr_base(B item); #5; item.p = 13; endtask
  task wr_local(D h); D item = h; #5; item.p = 14; endtask
  task wr_prop(); #5; item.p = 15; endtask
  task wr_chain(W w); #5; w.item.p = 16; endtask
  task wr_fork(D h); D item = h; fork begin #5; item.p = 18; end join endtask
  task wr_fork_formal(D item); fork begin #5; item.p = 19; end join endtask
endclass
module t;
  Blk k = new; T t = new;
  initial begin
    D a; B ab; W w; int v;
    a = new; fork t.wr_formal(a); #1 k.hold_b(a); join
    $display("T|formal|%0d %0d", a.p, a.bp());
    a = new; a.p = 7; fork t.rd_formal(a, v); #1 k.hold_b(a); join
    $display("T|read|%0d %0d", v, a.bp());
    a = new; ab = a; fork t.wr_base(ab); #1 k.hold_d(a); join
    $display("T|base|%0d %0d", a.p, a.bp());
    a = new; fork t.wr_local(a); #1 k.hold_b(a); join
    $display("T|local|%0d %0d", a.p, a.bp());
    a = new; t.item = a; fork t.wr_prop(); #1 k.hold_b(a); join
    $display("T|prop|%0d %0d", a.p, a.bp());
    a = new; w = new; w.item = a; fork t.wr_chain(w); #1 k.hold_b(a); join
    $display("T|chain|%0d %0d", a.p, a.bp());
    a = new; fork t.wr_fork(a); #1 k.hold_b(a); join
    $display("T|fork|%0d %0d", a.p, a.bp());
    a = new; fork t.wr_fork_formal(a); #1 k.hold_b(a); join
    $display("T|fork_formal|%0d %0d", a.p, a.bp());
  end
endmodule
"#;

#[test]
fn derived_handle_write_while_base_formal_suspended() {
    assert_eq!(t_lines(LOCAL_BASE), ["T|a|p=42 base_p=-1"]);
}

#[test]
fn shadowed_member_access_while_same_named_formal_suspended() {
    let want = [
        "T|formal|11 -1",
        "T|read|7 -1",
        "T|base|-2 13",
        "T|local|14 -1",
        "T|prop|15 -1",
        "T|chain|16 -1",
        "T|fork|18 -1",
        "T|fork_formal|19 -1",
    ];
    assert_eq!(t_lines(FAMILY), want);
}

// The same writes from module `initial` blocks: a fork child, a fork
// statement and a task local writing through an `automatic` or static
// block local, while another process waits in a task (a class method or a
// module task) whose formal of the base type has the same name.
const MODULE_BLOCKS: &str = r#"
class B; int p = -1; function int bp(); return p; endfunction endclass
class D extends B; int p = -2; endclass
class Blk; task hold(B item); #10; endtask endclass
module t;
  Blk k = new;
  D shared;
  task automatic mhold(B item); #10; endtask
  task automatic tk(); D item = new; fork begin #5; item.p = 23; end #1 k.hold(item); join
    $display("T|task_local|%0d %0d", item.p, item.bp()); endtask
  initial begin
    automatic D item = new;
    fork begin #5; item.p = 21; end #1 k.hold(item); join
    $display("T|fork_child|%0d %0d", item.p, item.bp());
  end
  initial begin
    automatic D item = new;
    #20;
    fork #5 item.p = 22; #1 k.hold(item); join
    $display("T|fork_stmt|%0d %0d", item.p, item.bp());
  end
  initial begin #40; tk(); end
  initial begin : a1
    D item;
    #60 item = new; shared = item;
    fork begin #5; item.p = 31; end join_none
    #8 $display("T|other_initial|%0d %0d", item.p, item.bp());
  end
  initial begin #61 k.hold(shared); end
  initial begin : a2
    automatic D item;
    #80 item = new;
    fork begin #5; item.p = 32; end #1 mhold(item); join
    $display("T|module_task|%0d %0d", item.p, item.bp());
  end
  initial begin : a3
    D item;
    #100 item = new;
    fork begin #5; item.p = 33; end #1 k.hold(item); join
    $display("T|static_local|%0d %0d", item.p, item.bp());
  end
endmodule
"#;

/// §8.14: the static type of a module block local decides the copy, as it
/// does for a class method's local (the case the PR left open).
#[test]
fn module_block_local_write_while_base_formal_suspended() {
    let want = [
        "T|fork_child|21 -1",
        "T|fork_stmt|22 -1",
        "T|task_local|23 -1",
        "T|other_initial|31 -1",
        "T|module_task|32 -1",
        "T|static_local|33 -1",
    ];
    assert_eq!(t_lines(MODULE_BLOCKS), want);
}
