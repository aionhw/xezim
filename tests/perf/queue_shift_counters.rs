//! IEEE 1800-2023 §7.10.2: `pop_front`, `push_front`, `insert` and `delete`
//! on a queue of plain elements move no element one at a time, whatever the
//! queue holds. The `queue_elem_moves` profile counter counts the elements a
//! shift moved one by one by name: it stays 0 for module, package, class,
//! static, handle, local, `ref` and instance queues of 1,500 elements, where
//! the element-by-element shift made about a million moves per loop. A queue
//! of unpacked structs keeps the member-wise move, and the counter sees it.
//! Expected `T|` lines were recorded from the reference simulator.
use std::process::Command;

fn run(tag: &str, src: &str) -> (Vec<String>, u64) {
    let dir = std::env::temp_dir().join(format!("xezim_qmoves_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, src).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines = text
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    let moves = text
        .lines()
        .find_map(|l| l.strip_prefix("[PROF] queue_elem_moves="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing queue_elem_moves:\n{text}"));
    (lines, moves)
}

#[test]
fn front_shifts_of_long_queues_move_no_element_by_name() {
    let (lines, moves) = run(
        "plain",
        r#"// §7.10.2: front and middle shifts of queues that stay long.
package pk;
  int pq[$];
  class Item; int id; function new(int i); id = i; endfunction endclass
endpackage
class Fifo;
  int q[$];
  static int sq[$];
  pk::Item hq[$];
  function int churn(int n);
    int acc = 0;
    int t;
    for (int i = 0; i < n; i++) q.push_back(i);
    for (int i = 0; i < n; i++) begin q.push_back(n + i); t = q.pop_front(); acc ^= t * (i + 1); end
    for (int i = 0; i < n / 4; i++) begin q.push_front(-i); t = q.pop_back(); acc += t; q.insert(n / 2, i); q.delete(n / 3); end
    return acc ^ q.size();
  endfunction
  static function int schurn(int n);
    int acc = 0;
    int t;
    for (int i = 0; i < n; i++) sq.push_front(i);
    for (int i = 0; i < n; i++) begin t = sq.pop_front(); acc = acc * 3 + t; end
    return acc;
  endfunction
  function int hchurn(int n);
    pk::Item it;
    int acc = 0;
    for (int i = 0; i < n; i++) begin it = new(i); hq.push_back(it); end
    for (int i = 0; i < n - 5; i++) begin it = hq.pop_front(); acc += it.id * (i & 7); end
    return acc + hq[0].id + hq.size();
  endfunction
endclass
module child;
  logic [63:0] q[$];
  string sq[$];
  function automatic int churn(int n);
    int acc = 0;
    logic [63:0] v;
    string s;
    for (int i = 0; i < n; i++) begin q.push_back(64'(i) * 64'd2654435761); sq.push_back($sformatf("%0d", i)); end
    for (int i = 0; i < n - 3; i++) begin v = q.pop_front(); acc ^= int'(v >> 7); s = sq.pop_front(); acc += s.len(); end
    return acc + q.size() + sq.size();
  endfunction
endmodule
module top;
  import pk::*;
  localparam int N = 1500;
  int mq[$];
  child u[2] ();
  Fifo f;
  function automatic int local_churn(int n);
    int lq[$];
    int acc = 0;
    int t;
    for (int i = 0; i < n; i++) lq.push_back(i ^ 5);
    for (int i = 0; i < n; i++) begin lq.push_front(i); t = lq.pop_back(); acc += t + lq[0] + lq[$]; end
    while (lq.size() > 0) begin t = lq.pop_front(); acc ^= t; end
    return acc;
  endfunction
  function automatic int ref_churn(ref int a[$], input int n);
    int acc = 0;
    int t;
    for (int i = 0; i < n; i++) begin t = a.pop_front(); acc += t * (1 + (i & 3)); end
    return acc;
  endfunction
  int acc, t;
  initial begin
    f = new;
    for (int i = 0; i < N; i++) mq.push_back(i * 7);
    for (int i = 0; i < N; i++) begin mq.push_back(i); t = mq.pop_front(); acc ^= t + i; end
    for (int i = 0; i < N; i++) pq.push_front(i);
    for (int i = 0; i < N - 1; i++) begin t = pq.pop_front(); acc += t; end
    $display("T| mod %0d size=%0d pkg=%0d", acc, mq.size(), pq.size());
    $display("T| ref %0d left=%0d", ref_churn(mq, N - 10), mq.size());
    $display("T| cls %0d", f.churn(N));
    $display("T| static %0d", Fifo::schurn(N));
    $display("T| handle %0d", f.hchurn(N));
    $display("T| local %0d", local_churn(N));
    $display("T| inst %0d %0d", u[1].churn(N), u[0].q.size());
  end
endmodule
"#,
    );
    assert_eq!(
        lines,
        [
            "T| mod 1124250 size=1500 pkg=1",
            "T| ref 2772146 left=10",
            "T| cls 1457968",
            "T| static 684127210",
            "T| handle 3912799",
            "T| local 3371280",
            "T| inst 450929218 0",
        ]
    );
    assert_eq!(moves, 0, "elements moved one by one");
}

#[test]
fn unpacked_struct_queue_keeps_the_member_wise_move() {
    let (lines, moves) = run(
        "struct",
        r#"module top;
  typedef struct { int id; string name; } rec_t;
  rec_t uq[$];
  int acc;
  initial begin
    for (int i = 0; i < 40; i++) uq.push_back('{id: i, name: $sformatf("n%0d", i)});
    for (int i = 0; i < 30; i++) begin rec_t r; r = uq.pop_front(); acc += r.id + r.name.len(); end
    uq.push_front('{id: 99, name: "front"});
    $display("T| %0d %0d %0d/%s %0d/%s %0d/%s", acc, uq.size(), uq[0].id, uq[0].name, uq[1].id, uq[1].name, uq[$].id, uq[$].name);
  end
endmodule
"#,
    );
    assert_eq!(lines, ["T| 515 11 99/front 30/n30 39/n39",]);
    // 30 pops of a queue shrinking from 40 elements, and one push_front of 10.
    assert_eq!(moves, (10..40).sum::<u64>() + 10);
}
