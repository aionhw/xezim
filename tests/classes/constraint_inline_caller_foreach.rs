//! §18.7 / §18.5.8.1: `foreach` in `obj.randomize() with { ... }` over an
//! array of the calling scope.
//!
//! An array name that does not resolve in the object's class resolves in the
//! scope of the randomize() call, and the foreach iterates that array's
//! elements. Such a loop over a non-empty caller array reached no solver and
//! was accepted unchecked, so draws picked the excluded values.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const CALLER_FOREACH: &str = r#"
class cfg_c;
  bit [4:0] held[];
  int grid[2][2];
  function new();
    held = {14, 18, 5};
    foreach (grid[i, j]) grid[i][j] = 3 + 2 * i + j;
  endfunction
endclass

class item_c;
  rand bit [4:0] x;
  rand bit [4:0] y;
  rand int data;
endclass

class runner_c;
  cfg_c cfg = new;
  bit [4:0] own[];
  bit [4:0] none[];

  function void run();
    item_c it = new;
    bit [4:0] q[$] = '{20, 21};
    int ok, bad;
    own = {1};

    ok = 0; bad = 0;
    for (int n = 0; n < 200; n++) begin
      if (!it.randomize() with {
            foreach (own[i]) { x != own[i]; }
            foreach (cfg.held[i]) { x != cfg.held[i]; }
          }) bad++;
      else if (!(it.x inside {14, 18, 5, 1})) ok++;
    end
    $display("A handle member and own array: ok=%0d fail=%0d", ok, bad);

    ok = 0; bad = 0;
    for (int n = 0; n < 200; n++) begin
      if (!it.randomize() with {
            foreach (q[i]) { x != q[i]; y != q[i] + 1; }
          }) bad++;
      else if (!(it.x inside {20, 21}) && !(it.y inside {21, 22})) ok++;
    end
    $display("B local queue: ok=%0d fail=%0d", ok, bad);

    ok = 0; bad = 0;
    for (int n = 0; n < 50; n++) begin
      if (!it.randomize() with {
            data inside {[0:9]};
            foreach (cfg.grid[i, j]) { data != cfg.grid[i][j] + i - j; }
          }) bad++;
      else if (!(it.data inside {3, 6})) ok++;
    end
    $display("C 2-D with indices: ok=%0d fail=%0d", ok, bad);

    ok = 0; bad = 0;
    for (int n = 0; n < 20; n++) begin
      if (!it.randomize() with {
            x == 7;
            foreach (none[i]) { x != none[i]; }
          }) bad++;
      else if (it.x == 7) ok++;
    end
    $display("D empty array: ok=%0d fail=%0d", ok, bad);
  endfunction
endclass

module top;
  initial begin
    runner_c r = new;
    r.run();
  end
endmodule
"#;

#[test]
fn inline_foreach_over_caller_arrays() {
    let got: Vec<String> = lines(CALLER_FOREACH)
        .into_iter()
        .filter(|l| l.contains(": ok="))
        .collect();
    let want = [
        "A handle member and own array: ok=200 fail=0",
        "B local queue: ok=200 fail=0",
        "C 2-D with indices: ok=50 fail=0",
        "D empty array: ok=20 fail=0",
    ];
    assert_eq!(got, want);
}
