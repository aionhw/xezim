//! §18.5.10: every solution of the constraints is a legal result, so a value
//! that few solutions allow still comes up. With `solve w, n before tot`, the
//! joint solver weighed each value of `k` by the solutions it leaves and also
//! counted the narrowing of `tot == w * (n + 1)`, a function of `w` and `n`
//! that holds one value per solution of its inputs: `K0`, `K2` and `flag`
//! were never drawn in 2000 calls (#1166).
use std::process::Command;

const DESIGN: &str = r#"
// Every combination of values that satisfies the constraints is a legal
// result of randomize() (IEEE 1800-2017 18.5.10). Here the enum value K1
// allows far more solutions than K0 or K2, and flag=1 far fewer than flag=0,
// so K0, K2 and flag=1 are rare, but over 2000 calls each must appear.
typedef enum bit [1:0] {K0 = 0, K1 = 1, K2 = 2} kind_e;

class item;
    rand kind_e       k;
    rand bit          flag;
    rand bit [7:0]    n;
    rand bit [2:0]    sz;
    rand bit [63:0]   a;
    rand int unsigned w;
    rand int unsigned tot;
    int unsigned      bus = 8;

    constraint c_k   {k inside {K0, K1, K2};}
    constraint c_w   {w == (1 << sz); w <= bus;}
    constraint c_tot {tot == w * (n + 1);}
    constraint c_n   {k == K2 -> n inside {1, 3, 7, 15}; k == K0 -> n <= 15;}
    constraint c_al  {k == K2 -> (a % w) == 0;}
    constraint c_4k  {k == K1 -> ((a - (a % w)) % 4096) + tot <= 4096; k == K2 -> tot <= 4096;}
    constraint c_f   {flag -> (tot inside {1, 2, 4, 8, 16, 32, 64, 128} && n <= 15 && (a % tot) == 0);}
    constraint c_ord {solve sz before w; solve w, n before tot;}
endclass

module top;
    initial begin
        item x = new;
        int  seen[3];
        int  flags = 0;
        int  fails = 0;
        repeat (2000) begin
            if (!x.randomize()) begin
                fails++;
                $display("FAIL randomize() returned 0");
                break;
            end
            seen[x.k]++;
            flags += x.flag;
        end
        $display("drawn: K0=%0d K1=%0d K2=%0d flag=1: %0d", seen[0], seen[1], seen[2], flags);
        if (seen[0] == 0) begin fails++; $display("FAIL K0 never drawn"); end
        if (seen[2] == 0) begin fails++; $display("FAIL K2 never drawn"); end
        if (flags == 0)   begin fails++; $display("FAIL flag=1 never drawn"); end
        $display("%s", fails ? "TEST FAILED" : "TEST PASSED");
        $finish;
    end
endmodule
"#;

#[test]
fn rare_constraint_branches_are_drawn() {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_rare_branches_{}_{}",
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
    assert!(log.contains("TEST PASSED"), "{log}");
}
