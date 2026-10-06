//! §8.11 / §18.5.14 / §18.6.1: a state value read through `this` (`this.lo`,
//! `this.lim.lo`) is the same property as `lo` / `lim.lo`, and a class that
//! also has a soft constraint must randomize the same with either spelling
//! (#256). The joint solver took `lo` and `lim.lo` as state, but a member path
//! through `this` that names no random variable reached `this` itself, which
//! it could not read, so the item was left out as unanalyzable. The solver
//! then gave up on its work budget, and — with a soft constraint present and
//! an array sized by the constraints — the trial loop kept rejecting the
//! assignments it had found that satisfied every constraint, so
//! `randomize()` returned 0.

use xezim::simulate;

const SRC: &str = r#"
class limits;
  int lo = 3;
  int n = 4;
endclass

class delay_cfg;
  int min_delay = 3;
  int max_delay = 3;
endclass

class cfg_t;
  delay_cfg wready = new();
endclass

// One level: a state member of `this` itself.
class own_member;
  int lo = 3;
  rand int d[];
  rand bit flag;
  constraint c_d {
    d.size() == 2;
    foreach (d[i]) {
      d[i] == this.lo;
    }
  }
  constraint c_soft { soft flag == 1; }
endclass

// The reported shape.
class one_level;
  limits lim = new();
  rand int d[];
  rand bit flag;
  constraint c_d {
    d.size() == 1;
    foreach (d[i]) {
      d[i] == this.lim.lo;
    }
  }
  constraint c_soft { soft flag == 1; }
endclass

// Two levels below `this`, under an if/else on state, with the size taken
// from a state variable.
class two_level;
  cfg_t configuration = new();
  int n = 8;
  rand int d[];
  rand bit flag;
  constraint c_d {
    d.size() == n;
    foreach (d[i]) {
      if (this.configuration.wready.max_delay > this.configuration.wready.min_delay) {
        d[i] inside {[this.configuration.wready.min_delay:this.configuration.wready.max_delay]};
      }
      else {
        d[i] == this.configuration.wready.min_delay;
      }
    }
  }
  constraint c_soft { soft flag == 1; }
endclass

// The size itself read through `this`.
class sized_by_path;
  limits lim = new();
  rand int d[];
  rand bit flag;
  constraint c_d {
    d.size() == this.lim.n;
    foreach (d[i]) {
      d[i] == this.lim.lo;
    }
  }
  constraint c_soft { soft flag == 1; }
endclass

// No array: already randomized correctly.
class scalar;
  limits lim = new();
  rand int x;
  rand bit flag;
  constraint c_x { x == this.lim.lo; }
  constraint c_soft { soft flag == 1; }
endclass

module top;
  function automatic string all_eq(int d[], int v);
    foreach (d[i]) if (d[i] != v) return "no";
    return "yes";
  endfunction

  initial begin
    own_member m = new();
    one_level a = new();
    two_level b = new();
    sized_by_path c = new();
    scalar s = new();
    int ok;
    ok = m.randomize();
    $display("own_member ok=%0d size=%0d all_lo=%s flag=%0d", ok, m.d.size(), all_eq(m.d, 3), m.flag);
    ok = a.randomize();
    $display("one_level ok=%0d size=%0d all_lo=%s flag=%0d", ok, a.d.size(), all_eq(a.d, 3), a.flag);
    ok = b.randomize();
    $display("two_level ok=%0d size=%0d all_lo=%s flag=%0d", ok, b.d.size(), all_eq(b.d, 3), b.flag);
    ok = c.randomize();
    $display("sized_by_path ok=%0d size=%0d all_lo=%s flag=%0d", ok, c.d.size(), all_eq(c.d, 3), c.flag);
    ok = s.randomize();
    $display("scalar ok=%0d x=%0d flag=%0d", ok, s.x, s.flag);
  end
endmodule
"#;

#[test]
fn this_path_state_with_soft_constraint_randomizes() {
    let out: Vec<String> = simulate(SRC, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect();
    assert_eq!(
        out,
        [
            "own_member ok=1 size=2 all_lo=yes flag=1",
            "one_level ok=1 size=1 all_lo=yes flag=1",
            "two_level ok=1 size=8 all_lo=yes flag=1",
            "sized_by_path ok=1 size=4 all_lo=yes flag=1",
            "scalar ok=1 x=3 flag=1",
        ],
        "{out:?}"
    );
}
