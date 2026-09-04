//! §6.12.2: a real assigned to an INTEGRAL variable converts by rounding.
//! Module variables did; a subroutine LOCAL (task, function, class method,
//! interface task) kept the real value, so `int div = freq / rate;` held
//! 32.55 and a later `count == div - 1` never became true — the uart AVIP's
//! baud clock never toggled and its driver stalled after one transaction.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
interface I;
  bit baudClk;
  task t(); int a; real r = 3.6; a = r; $display("IFACE %0d", a); endtask
  task gen(input real freq, input int rate, input int cycles);
    int div; static int count = 0; int toggles = 0;
    div = freq / rate;
    $display("DIV %0d", div);
    repeat (cycles) begin
      if (count == (div - 1)) begin count = 0; baudClk = ~baudClk; toggles++; end
      else count = count + 1;
    end
    $display("TOGGLES %0d", toggles);
  endtask
endinterface
class C; task t(); int a; real r = 3.6; a = r; $display("CLASS %0d", a); endtask endclass
module top;
  I i();
  int m; real rr = 3.6;
  task mt(); int a; a = rr; $display("MTASK %0d", a); endtask
  function automatic int f(); int a; a = rr; return a; endfunction
  initial begin
    C c = new;
    m = rr; $display("MODVAR %0d", m);
    i.t(); c.t(); mt(); $display("FUNC %0d", f());
    i.gen(5000.0, 149, 1000);   // 33.55 -> 34 -> 1000/34 = 29 toggles
    $finish;
  end
endmodule
"#;

fn run(jit: bool) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("real_to_integral_local");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(if jit { "t_jit.sv" } else { "t_default.sv" });
    std::fs::write(&sv, DESIGN).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--simulate", "-s", "top", "--no-cache", sv.to_str().unwrap()]);
    if jit {
        cmd.env("XEZIM_JIT", "1");
    }
    let output = cmd.output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

fn check(text: &str) {
    for want in ["MODVAR 4", "IFACE 4", "CLASS 4", "MTASK 4", "FUNC 4", "DIV 34", "TOGGLES 29"] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
}

#[test]
fn real_assigned_to_integral_locals_rounds() {
    check(&run(false));
}

#[test]
fn real_assigned_to_integral_locals_rounds_jit() {
    check(&run(true));
}
