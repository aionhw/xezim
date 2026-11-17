//! §18.4, §18.5.8.1, §18.5.9: a parent dynamic array whose size is tied to a
//! `rand` sub-object scalar (`pad.size() == 64 - (h.len + 18)`) must be solved
//! jointly with that sub-object in the size phase, and impossible exact sizes
//! must be rejected immediately rather than clamped to 65536 elements.

use std::process::Command;
use std::time::{Duration, Instant};
use xezim::simulate;

#[test]
fn min_a_and_200_calls_cover_legal_range() {
    let src = r#"
class hdr;
  rand bit [15:0] len;
  constraint len_c { len inside {[20:576]}; }
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == 64 - (h.len + 18);
  }
  function new(); h = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int fails = 0;
    int seen[int];
    for (int i = 0; i < 4; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL: randomize() returned 0 (iteration %0d)", i);
      end else if (!(p.h.len inside {[20:46]}) || p.pad.size() != 46 - p.h.len) begin
        fails++;
        $display("FAIL: bad solution len=%0d pad.size=%0d", p.h.len, p.pad.size());
      end else begin
        foreach (p.pad[j]) if (p.pad[j] !== 8'hFF) fails++;
        $display("ok len=%0d pad.size=%0d", p.h.len, p.pad.size());
      end
    end
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);

    for (int i = 0; i < 200; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL200: randomize() returned 0 (iteration %0d)", i);
        break;
      end
      if (!(p.h.len inside {[20:46]}) || p.pad.size() != 46 - p.h.len) begin
        fails++;
        $display("FAIL200: bad len=%0d pad.size=%0d", p.h.len, p.pad.size());
      end else begin
        foreach (p.pad[j]) if (p.pad[j] !== 8'hFF) fails++;
        seen[p.h.len] = 1;
      end
    end
    $display("COV fails=%0d distinct=%0d", fails, seen.num());
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 100_000).expect("simulate failed");
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "expected PASS, got:\n{lines:#?}");
    let cov = lines
        .iter()
        .find(|l| l.starts_with("COV "))
        .copied()
        .unwrap_or("");
    assert!(cov.starts_with("COV fails=0 "), "bad cov line: {lines:#?}");
    let distinct: usize = cov
        .split("distinct=")
        .nth(1)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    assert!(
        distinct >= 20,
        "expected >= 20 distinct legal len values across 200 calls, got {distinct}: {lines:#?}"
    );
}

#[test]
fn size_from_sub_object_without_foreach() {
    // Wide legal domain ensures unfixed rejection sampling cannot pass by luck.
    let src = r#"
class hdr;
  rand bit [31:0] len;
  constraint len_c { len inside {[20:100000]}; }
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  constraint pad_c {
    pad.size() == 64 - (h.len + 18);
  }
  function new(); h = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int fails = 0;
    for (int i = 0; i < 20; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL call=%0d", i);
        break;
      end else if (!(p.h.len inside {[20:46]}) || p.pad.size() != 46 - p.h.len) begin
        fails++;
        $display("FAIL bad len=%0d size=%0d", p.h.len, p.pad.size());
      end
    end
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 100_000).expect("simulate failed");
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:#?}");
}

#[test]
fn signed_size_expr_from_sub_object() {
    let src = r#"
class hdr;
  rand bit [15:0] len;
  constraint len_c { len inside {[20:576]}; }
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == (64 - int'(h.len + 18));
  }
  function new(); h = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int fails = 0;
    for (int i = 0; i < 20; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL call=%0d", i);
        break;
      end else if (!(p.h.len inside {[20:46]}) || p.pad.size() != 46 - p.h.len) begin
        fails++;
        $display("FAIL bad len=%0d size=%0d", p.h.len, p.pad.size());
      end else begin
        foreach (p.pad[j]) if (p.pad[j] !== 8'hFF) fails++;
      end
    end
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 100_000).expect("simulate failed");
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:#?}");
}

#[test]
fn size_tied_to_two_sub_objects() {
    let src = r#"
class part;
  rand bit [15:0] len;
  constraint len_c { len inside {[10:300]}; }
endclass
class pkt;
  rand part a;
  rand part b;
  rand bit [7:0] pad[];
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == 64 - (a.len + b.len + 18);
  }
  function new(); a = new(); b = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int fails = 0;
    for (int i = 0; i < 20; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL call=%0d", i);
        break;
      end else if (p.a.len < 10 || p.b.len < 10 || (p.a.len + p.b.len) > 46
                   || p.pad.size() != 46 - (p.a.len + p.b.len)) begin
        fails++;
        $display("FAIL bad a=%0d b=%0d size=%0d", p.a.len, p.b.len, p.pad.size());
      end else begin
        foreach (p.pad[j]) if (p.pad[j] !== 8'hFF) fails++;
      end
    end
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 100_000).expect("simulate failed");
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:#?}");
}

#[test]
fn size_from_rand_object_array_element() {
    let src = r#"
class hdr;
  rand bit [15:0] len;
  constraint len_c { len inside {[20:576]}; }
endclass
class pkt;
  rand hdr arr[2];
  rand bit [7:0] pad[];
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == 64 - (arr[1].len + 18);
  }
  function new(); arr[0] = new(); arr[1] = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int fails = 0;
    for (int i = 0; i < 20; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL call=%0d", i);
        break;
      end else if (!(p.arr[1].len inside {[20:46]}) || p.pad.size() != 46 - p.arr[1].len) begin
        fails++;
        $display("FAIL bad len=%0d size=%0d", p.arr[1].len, p.pad.size());
      end else begin
        foreach (p.pad[j]) if (p.pad[j] !== 8'hFF) fails++;
      end
    end
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 100_000).expect("simulate failed");
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:#?}");
}

#[test]
fn unsatisfiable_size_from_sub_object_fails_promptly() {
    let src = r#"
class hdr;
  rand bit [15:0] len;
  constraint len_c { len inside {[100:576]}; }
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == 64 - (h.len + 18);
  }
  function new(); h = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int ok;
    p.h.len = 25;
    p.pad = new[2];
    p.pad[0] = 8'h11;
    p.pad[1] = 8'h22;
    ok = p.randomize();
    $display("RES ok=%0d len=%0d size=%0d p0=%02x p1=%02x",
             ok, p.h.len, p.pad.size(), p.pad[0], p.pad[1]);
    $finish;
  end
endmodule
"#;
    let start = Instant::now();
    let res = simulate(src, 100_000).expect("simulate failed");
    let elapsed = start.elapsed();
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(
        lines.contains(&"RES ok=0 len=25 size=2 p0=11 p1=22"),
        "{lines:#?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "unsatisfiable size took too long: {elapsed:?}"
    );
}

#[test]
fn impossible_trial_size_skips_slow_clamping_and_reports_size_constraint_on_budget_exhaustion() {
    // Sub-object method call keeps the joint solver out (NotApplicable),
    // forcing trials through solve_array_sizes + take_randomize_budget.
    // Impossible Eq sizes must be rejected fast (<< 1s per 2 trials, not 8s)
    // and budget exhaustion must report `pad_c` rather than `use_correct_fcs_c`.
    let src = r#"
class hdr;
  rand bit [15:0] len;
  function int min_len(); return 20; endfunction
  constraint len_c { len >= min_len(); len <= 576; }
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  rand bit use_correct_fcs;
  constraint use_correct_fcs_c { use_correct_fcs == 1; }
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == 64 - (h.len + 18);
  }
  function new(); h = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int ok = p.randomize();
    $display("DONE ok=%0d fcs=%0d size=%0d", ok, p.use_correct_fcs, p.pad.size());
    $finish;
  end
endmodule
"#;
    let dir = std::env::temp_dir().join(format!("xezim_sub_size_budget_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("diag.sv");
    std::fs::write(&sv, src).unwrap();
    let start = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "--no-cache",
            sv.to_str().unwrap(),
            "+seed=1",
            "+xezim_rand_trials=2",
        ])
        .output()
        .expect("run xezim");
    let elapsed = start.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{:?}", out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        elapsed < Duration::from_secs(2),
        "budget run took too long ({elapsed:?}):\n{stderr}"
    );
    assert!(
        stdout.contains("DONE ok=0 fcs=0 size=0"),
        "expected restore on failure:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("randomize budget exhausted in class pkt"),
        "missing budget message:\n{stderr}"
    );
    assert!(
        stderr.contains("last failed constraints: pad_c"),
        "expected pad_c in last failed constraints:\n{stderr}"
    );
    assert!(
        !stderr.contains("use_correct_fcs_c"),
        "trivially satisfied use_correct_fcs_c should not be reported:\n{stderr}"
    );
}

#[test]
fn sub_object_array_elements_follow_parent_size_solve() {
    let src = r#"
class hdr;
  rand bit [7:0] len;
  rand bit [7:0] arr[2];
  constraint len_c { len inside {[1:200]}; }
  constraint arr_c { foreach (arr[i]) arr[i] == len; }
endclass
class pkt;
  rand hdr h;
  rand bit [7:0] pad[];
  constraint pad_c {
    pad.size() == 10 - h.len;
  }
  function new(); h = new(); endfunction
endclass
module top;
  initial begin
    pkt p = new();
    int fails = 0;
    for (int i = 0; i < 20; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL call=%0d", i);
        break;
      end else if (!(p.h.len inside {[1:10]}) || p.pad.size() != 10 - p.h.len
                   || p.h.arr[0] != p.h.len || p.h.arr[1] != p.h.len) begin
        fails++;
        $display("FAIL bad len=%0d size=%0d arr0=%0d arr1=%0d",
                 p.h.len, p.pad.size(), p.h.arr[0], p.h.arr[1]);
      end
    end
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);
    $finish;
  end
endmodule
"#;
    let res = simulate(src, 100_000).expect("simulate failed");
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:#?}");
}

#[test]
fn back_ref_with_nested_obj_array_and_dyn_size() {
    let src = r#"
typedef class pkt;
class policy_base;
  pkt item;
endclass
class policy_a extends policy_base;
  constraint c { item.h.len <= 40; }
endclass
class policy_unsat extends policy_base;
  constraint c { item.h.len < 20; }
endclass
class policy_list;
  rand policy_base q[$];
endclass
class hdr;
  rand bit [15:0] len;
  constraint len_c { len inside {[20:576]}; }
endclass
class pkt;
  rand hdr h;
  rand policy_list pl;
  rand bit [7:0] pad[];
  constraint pad_c {
    foreach (pad[i]) pad[i] == 8'hFF;
    pad.size() == 64 - (h.len + 18);
  }
  function new(bit fail = 0);
    policy_a pa = new();
    policy_unsat pu = new();
    h = new();
    pl = new();
    if (fail) pl.q = {pu}; else pl.q = {pa};
  endfunction
  function void pre_randomize();
    foreach (pl.q[i]) pl.q[i].item = this;
  endfunction
endclass
module top;
  initial begin
    pkt p = new(0);
    pkt bad = new(1);
    int fails = 0;
    int ok_bad;
    for (int i = 0; i < 20; i++) begin
      if (!p.randomize()) begin
        fails++;
        $display("FAIL call=%0d", i);
        break;
      end else if (!(p.h.len inside {[20:40]}) || p.pad.size() != 46 - p.h.len) begin
        fails++;
        $display("FAIL bad len=%0d size=%0d", p.h.len, p.pad.size());
      end else begin
        foreach (p.pad[j]) if (p.pad[j] !== 8'hFF) fails++;
      end
    end
    ok_bad = bad.randomize();
    if (ok_bad != 0) fails++;
    if (fails == 0) $display("PASS"); else $display("FAIL (%0d)", fails);
    $finish;
  end
endmodule
"#;
    let start = Instant::now();
    let res = simulate(src, 100_000).expect("simulate failed");
    let elapsed = start.elapsed();
    let lines: Vec<&str> = res.output.iter().map(|e| e.message.as_str()).collect();
    assert!(lines.contains(&"PASS"), "{lines:#?}");
    assert!(
        elapsed < Duration::from_secs(2),
        "back_ref_with_nested_obj_array_and_dyn_size took too long: {elapsed:?}"
    );
}
