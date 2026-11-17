//! §6.18, §15.3, §15.4: `mailbox` and `semaphore` are classes of package
//! `std`. A variable declared through a typedef of one (at module or package
//! scope, or a typedef of such a typedef) is constructed as one by `new()`;
//! the typedef at module scope is legal; an unassigned handle is null; and a
//! property reached through a chain of handles (`o.in.mb`) or named through
//! its class (`C::sm`) is constructed as a mailbox too. The expected lines are
//! the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
package tp;
  typedef mailbox #(int) pmbox_t;
endpackage
typedef mailbox #(int) cmbox_t;
typedef cmbox_t cmbox2_t;
typedef semaphore csem_t;
class inner_c; mailbox #(int) hmb; endclass
class outer_c; inner_c in = new; endclass
class hold_c;
  cmbox_t m;
  static cmbox_t sm;
  static mailbox #(int) sm2;
endclass
module top;
  cmbox_t a;
  cmbox2_t a2;
  tp::pmbox_t a3;
  csem_t s;
  mailbox #(int) mi;
  semaphore si;
  hold_c h = new;
  outer_c o = new;
  initial begin
    $display("null a=%0d a2=%0d s=%0d mi=%0d si=%0d h.m=%0d", a == null, a2 == null, s == null, mi == null, si == null, h.m == null);
    a = new(); a2 = new(); a3 = new(); s = new(1); h.m = new(); hold_c::sm = new(); hold_c::sm2 = new();
    o.in.hmb = new();
    a.put(1); a2.put(2); a2.put(3); a3.put(4); h.m.put(5); hold_c::sm.put(6); hold_c::sm2.put(7); o.in.hmb.put(8);
    $display("num a=%0d a2=%0d a3=%0d h.m=%0d sm=%0d sm2=%0d nested=%0d", a.num(), a2.num(), a3.num(), h.m.num(), hold_c::sm.num(), hold_c::sm2.num(), o.in.hmb.num());
    $display("sem %0d %0d", s.try_get(), s.try_get());
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "null a=1 a2=1 s=1 mi=1 si=1 h.m=1",
    "num a=1 a2=2 a3=1 h.m=1 sm=1 sm2=1 nested=1",
    "sem 1 0",
];

#[test]
fn typedef_of_builtin_class_declares_a_handle() {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_builtin_class_typedef_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut log = String::from_utf8_lossy(&out.stdout).to_string();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{log}");
    let got: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("null ") || l.starts_with("num ") || l.starts_with("sem "))
        .collect();
    assert_eq!(got, EXPECTED, "{log}");
}
