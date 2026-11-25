//! IEEE 1800-2023 §7.10.2: `pop_front`, `push_front`, `insert` and `delete`
//! move a queue's elements as one run instead of one by one. Every other
//! queue operation has to see the same queue afterwards: indexing on both
//! sides of the 64 placeholder element cells of a design-scope queue, writes,
//! slices, `foreach`, locator / reduction / ordering methods, whole-queue
//! copy, compare and concatenation, value and `ref` arguments, `%p`, and
//! processes sensitive to the size or the head. The same body runs on a
//! module, class-property, handle-qualified, task-local, package, static
//! class and instance-array queue. Expected `T|` lines were recorded from
//! the reference simulator.

use std::process::Command;

fn lines_until(src: &str, max_time: u64) -> Vec<String> {
    xezim::simulate(src, max_time)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

fn lines(src: &str) -> Vec<String> {
    lines_until(src, 100_000)
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Run `src` through the command line with `env`; its `T|` lines.
fn cli_lines(tag: &str, src: &str, args: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_qshift_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let mut c = Command::new(env!("CARGO_BIN_EXE_xezim"));
    c.args(["--no-cache"])
        .args(args)
        .arg(&path)
        .current_dir(&dir);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("run xezim");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "run failed:\n{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

/// The helpers a body calls: a queue by value and by reference (§13.5).
const FUNCS: &str = r#"  function automatic int by_value(int a[$]);
    int t = 0;
    void'(a.pop_front()); a.push_front(1);
    foreach (a[i]) t += a[i];
    return t;
  endfunction
  function automatic void by_ref(ref int a[$]);
    a[0] += 1000;
    a.push_front(-1);
    void'(a.pop_back());
  endfunction
"#;

/// Every queue operation, on the queue the macro `Q names.
const BODY_FULL: &str = r#"    // churn both ends across the 64-element mark
    for (int i = 0; i < 100; i++) `Q.push_back(i * 3 + 1);
    for (int i = 0; i < 30; i++) acc += `Q.pop_front();
    for (int i = 0; i < 20; i++) `Q.push_front(-(i + 1));
    for (int i = 0; i < 15; i++) begin `Q.push_back(1000 + i); acc ^= `Q.pop_front(); end
    for (int i = 0; i < 12; i++) begin `Q.push_front(2000 + i); acc += `Q.pop_back(); end
    $display("T| a size=%0d acc=%0d q0=%0d q1=%0d q63=%0d q64=%0d q65=%0d last=%0d", `Q.size(), acc, `Q[0], `Q[1], `Q[63], `Q[64], `Q[65], `Q[$]);
    // element writes on both sides of the mark, and one past the end (appends)
    `Q[0] = 5; `Q[63] = 6363; `Q[64] = 6464; `Q[$] = 9; `Q[`Q.size()] = 4242;
    $display("T| b size=%0d q0=%0d q63=%0d q64=%0d last=%0d prev=%0d", `Q.size(), `Q[0], `Q[63], `Q[64], `Q[$], `Q[$-1]);
    // an index past the end reads the default, also after a shrink
    void'(`Q.pop_back()); void'(`Q.pop_front());
    $display("T| c size=%0d past=%0d past2=%0d neg=%0d", `Q.size(), `Q[`Q.size()], `Q[`Q.size() + 3], `Q[-1]);
    // slices
    r = `Q[60:68];
    $display("T| d %p", r);
    r = `Q[0:3];
    $display("T| e %p", r);
    r = `Q[$-2:$];
    $display("T| f %p", r);
    // foreach
    acc = 0;
    foreach (`Q[i]) acc += `Q[i] * (i + 1);
    $display("T| g acc=%0d", acc);
    // insert / delete at the front, across the mark, at the end
    `Q.insert(0, 111); `Q.insert(64, 222); `Q.insert(30, 333); `Q.insert(`Q.size(), 444);
    `Q.delete(1); `Q.delete(70); `Q.delete(`Q.size() - 2); `Q.delete(0);
    $display("T| h size=%0d q0=%0d q29=%0d q30=%0d q62=%0d q63=%0d q64=%0d last=%0d", `Q.size(), `Q[0], `Q[29], `Q[30], `Q[62], `Q[63], `Q[64], `Q[$]);
    acc = 0;
    foreach (`Q[i]) acc = acc * 31 + `Q[i];
    $display("T| i acc=%0d", acc);
    // locator and reduction methods
    r = `Q.find(x) with (x > 1000);
    $display("T| j %p", r);
    r = `Q.find_index(x) with (x < -15);
    $display("T| k %p", r);
    r = `Q.find_first(x) with (x % 7 == 0);
    $display("T| l %p", r);
    r = `Q.find_last_index(x) with (x % 7 == 0);
    $display("T| m %p", r);
    r = `Q.min(); $display("T| n min=%p", r);
    r = `Q.max(); $display("T| o max=%p", r);
    $display("T| p sum=%0d xor=%0d", `Q.sum(), `Q.xor());
    r = `Q.unique();
    $display("T| q unique=%0d", r.size());
    // whole-queue copy, compare, concatenation
    r = `Q;
    $display("T| r eq=%0d size=%0d", r == `Q, r.size());
    acc = r.pop_front();
    r.push_front(acc + 1);
    $display("T| s eq=%0d ne=%0d r0=%0d q0=%0d", r == `Q, r != `Q, r[0], `Q[0]);
    r = {`Q[0:1], 77, `Q[$-1:$]};
    $display("T| t %p", r);
    `Q = {r, `Q[2:4]};
    $display("T| u %p", `Q);
    // ordering methods on a shifted queue
    for (int i = 0; i < 70; i++) `Q.push_back((i * 37) % 101);
    for (int i = 0; i < 5; i++) void'(`Q.pop_front());
    for (int i = 0; i < 3; i++) `Q.push_front(50 + i);
    `Q.reverse();
    $display("T| v size=%0d q0=%0d q1=%0d q63=%0d q64=%0d last=%0d", `Q.size(), `Q[0], `Q[1], `Q[63], `Q[64], `Q[$]);
    `Q.sort();
    $display("T| w q0=%0d q1=%0d q40=%0d q64=%0d last=%0d", `Q[0], `Q[1], `Q[40], `Q[64], `Q[$]);
    `Q.rsort();
    void'(`Q.pop_front()); `Q.push_front(-7);
    $display("T| x q0=%0d q1=%0d q40=%0d q64=%0d last=%0d sum=%0d", `Q[0], `Q[1], `Q[40], `Q[64], `Q[$], `Q.sum());
    // by value and by reference
    $display("T| y byval=%0d size=%0d q0=%0d", by_value(`Q), `Q.size(), `Q[0]);
    by_ref(`Q);
    $display("T| z size=%0d q0=%0d q1=%0d last=%0d", `Q.size(), `Q[0], `Q[1], `Q[$]);
    // emptying, and methods on an empty queue
    while (`Q.size() > 3) void'(`Q.pop_front());
    $display("T| A %p", `Q);
    `Q = {};
    `Q.push_front(8); `Q.push_front(9);
    $display("T| B size=%0d %p", `Q.size(), `Q);
    `Q.delete();
    $display("T| C size=%0d", `Q.size());
    for (int i = 0; i < 200; i++) `Q.push_front(i);
    for (int i = 0; i < 199; i++) void'(`Q.pop_back());
    $display("T| D size=%0d q0=%0d", `Q.size(), `Q[0]);
"#;

/// The shifting operations and what reads them back, in the forms every
/// owner kind supports.
const BODY_CORE: &str = r#"    // churn both ends across the 64-element mark
    for (int i = 0; i < 100; i++) `Q.push_back(i * 3 + 1);
    for (int i = 0; i < 30; i++) acc += `Q.pop_front();
    for (int i = 0; i < 20; i++) `Q.push_front(-(i + 1));
    for (int i = 0; i < 15; i++) begin `Q.push_back(1000 + i); acc ^= `Q.pop_front(); end
    for (int i = 0; i < 12; i++) begin `Q.push_front(2000 + i); acc += `Q.pop_back(); end
    $display("T| a size=%0d acc=%0d q0=%0d q1=%0d q63=%0d q64=%0d q65=%0d last=%0d", `Q.size(), acc, `Q[0], `Q[1], `Q[63], `Q[64], `Q[65], `Q[$]);
    `Q[0] = 5; `Q[63] = 6363; `Q[64] = 6464; `Q[$] = 9;
    void'(`Q.pop_back()); void'(`Q.pop_front());
    $display("T| c size=%0d q0=%0d q62=%0d q63=%0d last=%0d past=%0d", `Q.size(), `Q[0], `Q[62], `Q[63], `Q[$], `Q[`Q.size()]);
    acc = 0;
    foreach (`Q[i]) acc += `Q[i] * (i + 1);
    $display("T| g acc=%0d", acc);
    `Q.insert(0, 111); `Q.insert(64, 222); `Q.insert(30, 333); `Q.insert(`Q.size(), 444);
    `Q.delete(1); `Q.delete(70); `Q.delete(`Q.size() - 2); `Q.delete(0);
    $display("T| h size=%0d q0=%0d q29=%0d q30=%0d q62=%0d q63=%0d q64=%0d last=%0d", `Q.size(), `Q[0], `Q[29], `Q[30], `Q[62], `Q[63], `Q[64], `Q[$]);
    acc = 0;
    for (int i = 0; i < `Q.size(); i++) acc = acc * 31 + `Q[i];
    $display("T| i acc=%0d sum=%0d", acc, `Q.sum());
    for (int i = 0; i < 70; i++) `Q.push_back((i * 37) % 101);
    for (int i = 0; i < 5; i++) void'(`Q.pop_front());
    for (int i = 0; i < 3; i++) `Q.push_front(50 + i);
    `Q.reverse();
    $display("T| v size=%0d q0=%0d q1=%0d q63=%0d q64=%0d last=%0d", `Q.size(), `Q[0], `Q[1], `Q[63], `Q[64], `Q[$]);
    `Q.sort();
    $display("T| w q0=%0d q1=%0d q40=%0d q64=%0d last=%0d", `Q[0], `Q[1], `Q[40], `Q[64], `Q[$]);
    `Q.rsort();
    void'(`Q.pop_front()); `Q.push_front(-7);
    $display("T| x q0=%0d q1=%0d q40=%0d q64=%0d last=%0d sum=%0d", `Q[0], `Q[1], `Q[40], `Q[64], `Q[$], `Q.sum());
    while (`Q.size() > 3) void'(`Q.pop_front());
    $display("T| A size=%0d %0d %0d %0d", `Q.size(), `Q[0], `Q[1], `Q[2]);
    `Q.delete();
    `Q.push_front(8); `Q.push_front(9);
    $display("T| B size=%0d %0d %0d", `Q.size(), `Q[0], `Q[1]);
    for (int i = 0; i < 200; i++) `Q.push_front(i);
    for (int i = 0; i < 199; i++) void'(`Q.pop_back());
    $display("T| D size=%0d q0=%0d q1=%0d q2=%0d", `Q.size(), `Q[0], `Q[1], `Q[2]);
"#;

/// `body` on the queue of one owner kind.
fn design(owner: &str, body: &str) -> String {
    let template = match owner {
        "mod" => {
            r#"`define Q q
module top;
  int q[$];
  int r[$];
  int acc;
@FUNCS@  initial begin
@BODY@  end
endmodule
"#
        }
        "cls" => {
            r#"`define Q q
class C;
  int q[$];
  int r[$];
  int acc;
@FUNCS@  task run();
@BODY@  endtask
endclass
module top;
  C c;
  initial begin c = new; c.run(); end
endmodule
"#
        }
        "cext" => {
            r#"`define Q c.q
class C;
  int q[$];
endclass
module top;
  C c;
  int r[$];
  int acc;
@FUNCS@  initial begin
    c = new;
@BODY@  end
endmodule
"#
        }
        "local" => {
            r#"`define Q q
module top;
@FUNCS@  task automatic run();
    int q[$];
    int r[$];
    int acc;
@BODY@  endtask
  initial run();
endmodule
"#
        }
        "pkg" => {
            r#"`define Q p::q
package p;
  int q[$];
endpackage
module top;
  int r[$];
  int acc;
@FUNCS@  initial begin
@BODY@  end
endmodule
"#
        }
        "static" => {
            r#"`define Q q
class C;
  static int q[$];
  static int r[$];
  static int acc;
  static task run();
@BODY@  endtask
endclass
module top;
  initial C::run();
endmodule
"#
        }
        "inst" => {
            r#"`define Q q
module child;
  int q[$];
  int r[$];
  int acc;
@FUNCS@  task run();
@BODY@  endtask
endmodule
module top;
  child u[2] ();
  initial begin u[1].run(); $display("T| other size=%0d", u[0].q.size()); end
endmodule
"#
        }
        _ => panic!("owner kind {owner}"),
    };
    template.replace("@FUNCS@", FUNCS).replace("@BODY@", body)
}

fn full_expected() -> Vec<String> {
    strs(&[
        "T| a size=90 acc=10770 q0=2011 q1=2010 q63=229 q64=232 q65=235 last=1002",
        "T| b size=91 q0=5 q63=6363 q64=6464 last=4242 prev=9",
        "T| c size=89 past=0 past2=0 neg=0",
        "T| d '{223, 226, 6363, 6464, 235, 238, 241, 244, 247}",
        "T| e '{2010, 2009, 2008, 2007}",
        "T| f '{1000, 1001, 9}",
        "T| g acc=1880104",
        "T| h size=89 q0=2009 q29=130 q30=133 q62=6363 q63=222 q64=6464 last=444",
        "T| i acc=-516390052",
        "T| j '{2009, 2008, 2007, 2006, 2005, 2004, 2003, 2002, 2001, 2000, 6363, 6464, 1001}",
        "T| k '{}",
        "T| l '{2009}",
        "T| m '{87}",
        "T| n min='{-5}",
        "T| o max='{6464}",
        "T| p sum=48764 xor=-72",
        "T| q unique=89",
        "T| r eq=1 size=89",
        "T| s eq=0 ne=1 r0=2010 q0=2009",
        "T| t '{2009, 2008, 77, 1001, 444}",
        "T| u '{2009, 2008, 77, 1001, 444, 2007, 2006, 2005}",
        "T| v size=76 q0=28 q1=92 q63=20 q64=84 last=52",
        "T| w q0=0 q1=2 q40=53 q64=89 last=2007",
        "T| x q0=-7 q1=2006 q40=50 q64=16 last=0 sum=7662",
        "T| y byval=7670 size=76 q0=-7",
        "T| z size=76 q0=-1 q1=993 last=2",
        "T| A '{5, 3, 2}",
        "T| B size=2 '{9, 8}",
        "T| C size=0",
        "T| D size=1 q0=199",
    ])
}

fn core_expected() -> Vec<String> {
    strs(&[
        "T| a size=90 acc=10770 q0=2011 q1=2010 q63=229 q64=232 q65=235 last=1002",
        "T| c size=88 q0=2010 q62=6363 q63=6464 last=1001 past=0",
        "T| g acc=1879303",
        "T| h size=88 q0=2009 q29=130 q30=133 q62=6363 q63=222 q64=6464 last=444",
        "T| i acc=2061551667 sum=47763",
        "T| v size=156 q0=28 q1=92 q63=20 q64=84 last=52",
        "T| w q0=-5 q1=-4 q40=50 q64=82 last=6464",
        "T| x q0=-7 q1=6363 q40=202 q64=130 last=-5 sum=34915",
        "T| A size=3 -3 -4 -5",
        "T| B size=2 9 8",
        "T| D size=3 q0=199 q1=198 q2=197",
    ])
}

/// A module-scope queue: its first 64 elements are table cells, the rest a
/// run, and every shift crosses that mark.
#[test]
fn module_queue_keeps_every_operation_after_front_shifts() {
    assert_eq!(lines(&design("mod", BODY_FULL)), full_expected());
}

/// A class-property queue, used from the class's own task.
#[test]
fn class_property_queue_keeps_every_operation_after_front_shifts() {
    assert_eq!(lines(&design("cls", BODY_FULL)), full_expected());
}

/// The interpreted method executor shifts the same way as the compiled one.
#[test]
fn interpreted_methods_shift_the_same_way() {
    let env = [("XEZIM_COMPILE_METHODS", "0")];
    assert_eq!(
        cli_lines("interp_cls", &design("cls", BODY_FULL), &[], &env),
        full_expected()
    );
    assert_eq!(
        cli_lines("interp_static", &design("static", BODY_CORE), &[], &env),
        core_expected()
    );
}

/// A queue reached through a handle, a task-local automatic queue, a package
/// queue, a static class queue and the queue of one instance of an array.
#[test]
fn every_owner_kind_shifts_its_queue() {
    for owner in ["cext", "local", "pkg", "static"] {
        assert_eq!(lines(&design(owner, BODY_CORE)), core_expected(), "{owner}");
    }
    let mut inst = core_expected();
    inst.push("T| other size=0".to_string());
    assert_eq!(lines(&design("inst", BODY_CORE)), inst);
}

/// Strings, reals, wide 4-state vectors, packed structs, enums, class
/// handles and a bounded queue move as a run; unpacked structs, fixed-array
/// elements and queue rows keep their member-wise move.
#[test]
fn element_kinds_shift_with_their_queue() {
    let got = lines(
        r#"// Element kinds and containers around the front-shift path (§7.10.2).
package pk;
  typedef struct packed { logic [7:0] tag; logic [15:0] data; } pkt_t;
  typedef struct { int id; string name; } rec_t;
  typedef enum { RED, GREEN, BLUE } col_t;
  class Item;
    int id;
    function new(int i); id = i; endfunction
  endclass
endpackage
module top;
  import pk::*;
  string  sq[$];
  real    rq[$];
  logic [127:0] wq[$];
  pkt_t   pq[$];
  col_t   eq[$];
  Item    hq[$];
  int     bq[$:9];
  rec_t   uq[$];
  int     fq[$][2];
  int     qq[$][$];
  int     aq[string][$];
  int     qa[3][$];
  byte    yq[$];
  logic [3:0] xq[$];
  int     n;
  string  s;

  initial begin
    // strings, across the 64-element mark
    for (int i = 0; i < 70; i++) sq.push_back($sformatf("s%0d", i));
    for (int i = 0; i < 5; i++) s = {s, sq.pop_front()};
    sq.push_front("front"); sq.insert(64, "mid"); sq.delete(1);
    $display("T| str %s size=%0d q0=%s q1=%s q62=%s q63=%s q64=%s last=%s", s, sq.size(), sq[0], sq[1], sq[62], sq[63], sq[64], sq[$]);
    // reals
    for (int i = 0; i < 70; i++) rq.push_back(i * 0.5);
    void'(rq.pop_front()); rq.push_front(-1.25); rq.push_front(2.5); rq.delete(66);
    $display("T| real size=%0d %0.2f %0.2f %0.2f %0.2f %0.2f", rq.size(), rq[0], rq[1], rq[63], rq[64], rq[$]);
    // elements wider than 64 bits, with x and z bits
    for (int i = 0; i < 70; i++) wq.push_back({64'(i), 64'hFFFF_0000_FFFF_0000 + 64'(i)});
    wq.push_front(128'hx); wq.push_front({64'hz, 64'h1});
    void'(wq.pop_back());
    $display("T| wide size=%0d %h %h %h %h", wq.size(), wq[0], wq[1], wq[64], wq[$]);
    $display("T| wide pop %h %h", wq.pop_front(), wq.pop_front());
    // packed structs and enums
    for (int i = 0; i < 70; i++) pq.push_back('{tag: 8'(i), data: 16'(i * 257)});
    for (int i = 0; i < 3; i++) void'(pq.pop_front());
    pq.push_front('{tag: 8'hAA, data: 16'h5555});
    $display("T| pkt size=%0d %h.%h %h.%h %h.%h", pq.size(), pq[0].tag, pq[0].data, pq[64].tag, pq[64].data, pq[$].tag, pq[$].data);
    for (int i = 0; i < 70; i++) eq.push_back(col_t'(i % 3));
    void'(eq.pop_front()); eq.push_front(BLUE); eq.insert(65, GREEN);
    $display("T| enum size=%0d %s %s %s %s %s", eq.size(), eq[0].name(), eq[1].name(), eq[64].name(), eq[65].name(), eq[$].name());
    // class handles
    for (int i = 0; i < 70; i++) begin Item it; it = new(i); hq.push_back(it); end
    n = 0;
    for (int i = 0; i < 10; i++) begin Item it; it = hq.pop_front(); n += it.id; end
    begin Item it; it = new(900); hq.push_front(it); end
    hq.delete(64);
    $display("T| handle n=%0d size=%0d %0d %0d %0d %0d null=%0d", n, hq.size(), hq[0].id, hq[1].id, hq[59].id, hq[$].id, hq[0] == null);
    // §7.10.5: a bounded queue drops the highest index after each insertion
    for (int i = 0; i < 14; i++) bq.push_front(i);
    $display("T| bound size=%0d %p", bq.size(), bq);
    void'(bq.pop_front()); bq.push_back(77); bq.push_back(78);
    $display("T| bound size=%0d %p", bq.size(), bq);
    // unpacked structs
    for (int i = 0; i < 6; i++) uq.push_back('{id: i, name: $sformatf("n%0d", i)});
    void'(uq.pop_front()); uq.push_front('{id: 50, name: "fifty"}); uq.delete(2);
    uq.insert(1, '{id: 60, name: "sixty"});
    $display("T| rec size=%0d %0d/%s %0d/%s %0d/%s %0d/%s", uq.size(), uq[0].id, uq[0].name, uq[1].id, uq[1].name, uq[2].id, uq[2].name, uq[$].id, uq[$].name);
    // fixed-array elements
    for (int i = 0; i < 5; i++) fq.push_back('{i, i * 10});
    void'(fq.pop_front()); fq.push_front('{7, 70}); fq.delete(1);
    $display("T| fixed size=%0d %0d,%0d %0d,%0d %0d,%0d", fq.size(), fq[0][0], fq[0][1], fq[1][0], fq[1][1], fq[3][0], fq[3][1]);
    // rows that are queues themselves
    aq["a"].push_back(1); aq["a"].push_back(2); aq["a"].push_back(3); aq["b"].push_back(9);
    aq["a"].push_front(0); n = aq["a"].pop_back(); aq["a"].insert(1, 5); aq["b"].push_front(8);
    $display("T| assocq n=%0d a=%p b=%p", n, aq["a"], aq["b"]);
    for (int i = 0; i < 70; i++) qa[1].push_back(i);
    for (int i = 0; i < 4; i++) void'(qa[1].pop_front());
    qa[1].push_front(-1); qa[0].push_front(3); qa[0].push_front(2); void'(qa[0].pop_front());
    $display("T| arrq %0d %0d %0d %0d %0d | %p | %0d", qa[1].size(), qa[1][0], qa[1][1], qa[1][64], qa[1][$], qa[0], qa[2].size());
    // signed and 4-state elements keep their type
    for (int i = 0; i < 70; i++) yq.push_back(8'(i * 5));
    void'(yq.pop_front()); yq.push_front(-3);
    for (int i = 0; i < 70; i++) xq.push_back(i[3:0]);
    xq.push_front(4'b1x0z); void'(xq.pop_back());
    $display("T| typed %0d %0d %0d %b %b %b", yq[0], yq[30], yq[65], xq[0], xq[1], xq[65]);
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        strs(&[
            "T| str s0s1s2s3s4 size=66 q0=front q1=s6 q62=s67 q63=mid q64=s68 last=s69",
            "T| real size=70 2.50 -1.25 31.00 31.50 34.50",
            "T| wide size=71 zzzzzzzzzzzzzzzz0000000000000001 xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx 000000000000003effff0000ffff003e 0000000000000044ffff0000ffff0044",
            "T| wide pop zzzzzzzzzzzzzzzz0000000000000001 xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
            "T| pkt size=68 aa.5555 42.4242 45.4545",
            "T| enum size=71 BLUE GREEN GREEN GREEN RED",
            "T| handle n=45 size=61 900 10 68 69 null=0",
            "T| bound size=10 '{13, 12, 11, 10, 9, 8, 7, 6, 5, 4}",
            "T| bound size=10 '{12, 11, 10, 9, 8, 7, 6, 5, 4, 77}",
            "T| rec size=6 50/fifty 60/sixty 1/n1 5/n5",
            "T| fixed size=4 7,70 2,20 4,40",
            "T| assocq n=3 a='{0, 5, 1, 2} b='{8, 9}",
            "T| arrq 67 -1 4 67 69 | '{3} | 0",
            "T| typed -3 -106 69 1x0z 0000 0000",
        ])
    );
}

/// §9.2.2.2 / §9.4.3: `always_comb` readers of `q.size()` and `q[0]` and
/// parked `wait (q.size() ...)` conditions follow every shift.
#[test]
fn size_and_head_readers_follow_front_shifts() {
    let got = lines(
        r#"// §9.2.2.2 / §9.4.3: processes sensitive to a queue's size and head see every front shift.
class Chan;
  int q[$];
  int got[$];
  task consume(int n);
    repeat (n) begin
      wait (q.size() > 0);
      got.push_back(q.pop_front());
      #1;
    end
  endtask
endclass
module top;
  int q[$];
  int seen_size, head, wakes;
  logic clk = 0;
  Chan c;
  always #5 clk = ~clk;
  always_comb seen_size = q.size();
  always_comb head = (q.size() > 0) ? q[0] : -1;
  always @(seen_size) wakes++;
  initial begin
    c = new;
    for (int i = 0; i < 80; i++) q.push_back(i);
    #1 $display("T| a size=%0d head=%0d", seen_size, head);
    void'(q.pop_front());
    #1 $display("T| b size=%0d head=%0d", seen_size, head);
    q.push_front(500);
    #1 $display("T| c size=%0d head=%0d", seen_size, head);
    q.insert(0, 7);
    #1 $display("T| d size=%0d head=%0d", seen_size, head);
    q.delete(0); q[0] = 42;
    #1 $display("T| e size=%0d head=%0d", seen_size, head);
    q.delete(40);
    #1 $display("T| f size=%0d head=%0d wakes=%0d", seen_size, head, wakes);
    fork
      begin wait (q.size() == 0); $display("T| g empty at %0t head=%0d", $time, head); end
      begin wait (q.size() == 70 && q[0] == 9); $display("T| h at %0t", $time); end
    join_none
    repeat (100) @(posedge clk) if (q.size() > 0) void'(q.pop_front());
    $display("T| i size=%0d head=%0d wakes=%0d", seen_size, head, wakes);
    // a consumer parked on a class queue
    fork c.consume(6); join_none
    #3 c.q.push_back(11);
    #3 c.q.push_back(12); c.q.push_back(13); c.q.push_front(10);
    #9 c.q.push_front(14); c.q.push_front(15);
    #20 $display("T| j %p left=%0d", c.got, c.q.size());
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        strs(&[
            "T| a size=80 head=0",
            "T| b size=79 head=1",
            "T| c size=80 head=500",
            "T| d size=81 head=7",
            "T| e size=80 head=42",
            "T| f size=79 head=42 wakes=6",
            "T| h at 95",
            "T| g empty at 795 head=-1",
            "T| i size=0 head=-1 wakes=85",
            "T| j '{11, 10, 12, 13, 15, 14} left=0",
        ])
    );
}

/// §8.7: an object only a queue refers to stays alive across the collector
/// while the queue shifts; a popped and dropped one is not kept by it.
#[test]
fn queued_objects_survive_collection_while_shifting() {
    let got = lines(
        r#"// §8.7: objects held only by a queue stay alive while other objects are created and dropped.
class Node;
  int id;
  Node peer;
  int pad[8];
  function new(int i); id = i; endfunction
endclass
class Pool;
  Node live[$];
  Node scratch[$];
  function int churn(int rounds);
    int bad = 0;
    for (int r = 0; r < rounds; r++) begin
      Node n = new(r);
      Node t = new(-r);
      n.peer = t;
      live.push_back(n);
      scratch.push_front(t);
      if (live.size() > 90) begin
        Node o = live.pop_front();
        if (o.id != r - 90 || o.peer.id != -(r - 90)) bad++;
      end
      if (scratch.size() > 3) void'(scratch.pop_back());
      if (r % 97 == 0) begin live.insert(live.size() / 2, n); live.delete(live.size() / 2); end
    end
    return bad;
  endfunction
endclass
module top;
  Pool p;
  Node mq[$];
  int bad, sum;
  initial begin
    p = new;
    for (int r = 0; r < 12000; r++) begin
      Node n;
      n = new(r);
      mq.push_back(n);
      if (mq.size() > 70) begin
        Node o;
        o = mq.pop_front();
        if (o.id != r - 70) bad++;
      end
    end
    foreach (mq[i]) sum += mq[i].id;
    $display("T| mod bad=%0d size=%0d sum=%0d head=%0d", bad, mq.size(), sum, mq[0].id);
    bad = p.churn(12000);
    sum = 0;
    foreach (p.live[i]) sum += p.live[i].id + p.live[i].peer.id;
    $display("T| cls bad=%0d size=%0d sum=%0d head=%0d tail=%0d scratch=%0d", bad, p.live.size(), sum, p.live[0].id, p.live[$].peer.id, p.scratch.size());
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        strs(&[
            "T| mod bad=0 size=70 sum=837515 head=11930",
            "T| cls bad=0 size=90 sum=0 head=11910 tail=-11999 scratch=3",
        ])
    );
}

/// A checkpoint taken while queues hold shifted elements restores them:
/// the restarted run prints what the uninterrupted one prints.
#[test]
fn checkpoint_restores_shifted_queues() {
    let dir = std::env::temp_dir().join(format!("xezim_qshift_ckpt_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let ck = dir.join("s.ckpt");
    let plain = r#"module top;
  int q[$];
  class C; int q[$]; string s[$]; endclass
  C c;
  int acc;
  initial begin
    c = new;
    for (int i = 0; i < 150; i++) begin q.push_back(i); c.q.push_back(i * 2); c.s.push_back($sformatf("s%0d", i)); end
    for (int i = 0; i < 20; i++) begin void'(q.pop_front()); void'(c.q.pop_front()); void'(c.s.pop_front()); end
    q.push_front(-1); c.q.push_front(-2); void'(q.pop_back());
  end
  initial begin
    #20;
    for (int i = 0; i < 50; i++) begin acc += q.pop_front(); acc ^= c.q.pop_front(); q.push_back(i); end
    q.insert(64, 5); c.q.delete(3); c.s.push_front("x");
    $display("T| %0d %0d %0d %0d %0d %0d %0d %s %s %0d", acc, q.size(), q[0], q[63], q[64], q[$], c.q[0], c.s[0], c.s[1], c.s.size());
    $finish;
  end
endmodule
"#;
    let saving = plain
        .replace(
            "    c = new;",
            &format!("    if ($test$plusargs(\"again\")) $restart(\"{}\");\n    else begin\n    c = new;", ck.display()),
        )
        .replace(
            "void'(q.pop_back());\n  end",
            &format!("void'(q.pop_back());\n    #10 $save(\"{}\");\n    end\n  end", ck.display()),
        );
    assert!(saving.contains("$save") && saving.contains("$restart"));
    let expected = strs(&["T| 2709 131 69 132 5 49 138 x s20 131"]);
    assert_eq!(lines(plain), expected);
    assert_eq!(cli_lines("ckpt_save", &saving, &[], &[]), expected);
    assert_eq!(cli_lines("ckpt_again", &saving, &["+again"], &[]), expected);
    let _ = std::fs::remove_dir_all(&dir);
}
