//! §7.2.1/§7.10 — dynamic collections nested inside aggregates:
//! queue-of-queue push_back, struct dynamic/queue members (new[n], size,
//! copy-by-value on assign), and queue-element member queues
//! (`pq[0].data.push_back`). All reference-validated (audit round I1).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn queue_of_queue_push_back() {
    let src = r#"
module tb;
  int q[$][$];
  int tmp[$];
  int sz, e0, e1, after_val;
  initial begin
    tmp.push_back(1); tmp.push_back(2);
    q.push_back(tmp);
    sz = q[0].size(); e0 = q[0][0]; e1 = q[0][1];
    tmp.push_back(3);
    after_val = q[0].size(); // value copy: outer row unaffected
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 2, "inner row size");
    assert_eq!(u(&sim, "e0"), 1);
    assert_eq!(u(&sim, "e1"), 2);
    assert_eq!(u(&sim, "after_val"), 2, "outer row unaffected by later source push");
}

#[test]
fn struct_dynamic_member_new_and_copy() {
    let src = r#"
module tb;
  typedef struct { int k; int d[]; } S;
  typedef struct { int d[]; } D; // only member is dynamic
  S a, b;
  D x, y;
  int asz, bsz, bk, b0, ysz;
  initial begin
    a.k = 9;
    a.d = new[1];
    a.d[0] = 1;
    asz = a.d.size();
    b = a;
    bsz = b.d.size(); bk = b.k; b0 = b.d[0];
    x.d = new[2]; x.d[0] = 5; x.d[1] = 6;
    y = x;
    ysz = y.d.size();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "asz"), 1, "new[1] on struct member sets size");
    assert_eq!(u(&sim, "bsz"), 1, "assign copies dynamic member size");
    assert_eq!(u(&sim, "bk"), 9);
    assert_eq!(u(&sim, "b0"), 1, "assign copies dynamic member elements");
    assert_eq!(u(&sim, "ysz"), 2, "struct whose only member is dynamic");
}

#[test]
fn struct_member_queue_ops() {
    let src = r#"
module tb;
  typedef struct { int id; int q[$]; } P;
  P p;
  int sz, q0;
  initial begin
    p.id = 3;
    p.q.push_back(7);
    sz = p.q.size(); q0 = p.q[0];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 1, "push_back on struct member queue");
    assert_eq!(u(&sim, "q0"), 7);
}

#[test]
fn queue_element_member_queue() {
    let src = r#"
module tb;
  typedef struct { int id; int data[$]; } Pkt;
  Pkt pq[$];
  Pkt p;
  int sz, d0, d1;
  initial begin
    p.id = 1; p.data.push_back(11);
    pq.push_back(p);
    pq[0].data.push_back(13);
    sz = pq[0].data.size(); d0 = pq[0].data[0]; d1 = pq[0].data[1];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "sz"), 2, "push_back through indexed element member");
    assert_eq!(u(&sim, "d0"), 11, "copied from pushed struct");
    assert_eq!(u(&sim, "d1"), 13, "appended after the copy");
}
