//! §18.4: the rand members of the unpacked-struct elements of a rand
//! dynamic array are random variables. They were never drawn, so every
//! element kept its initial value and a constraint such as
//! `offset inside {[1:mx]}` made randomize() fail.
#[test]
fn rand_members_of_struct_array_elements_are_solved() {
    let result = xezim::simulate(
        r#"
typedef enum bit [2:0] { M_A = 1, M_B = 5 } mode_t;
typedef struct {
  rand bit [31:0] offset;
  rand integer    level;
  rand mode_t     mode;
  bit [1:0]       zero;
} cfg_t;
class c;
  rand bit hi;
  rand cfg_t a[];
  bit [31:0] mx = 32'h40000;
  constraint k {
    foreach (a[i]) {
      if (i != 0) { a[i].offset inside {[1 : mx]}; } else { a[i].offset == 0; }
      a[i].level >= 0;
      if (hi) { a[i].level <= 32; } else { a[i].level <= 29; }
      (a[i].mode == M_B) -> (a[i].level > 3);
    }
  }
  function new(); a = new[4]; endfunction
  function bit valid();
    foreach (a[i]) begin
      if (i == 0 ? a[i].offset != 0 : !(a[i].offset inside {[1 : mx]})) return 0;
      if (a[i].level < 0 || a[i].level > (hi ? 32 : 29)) return 0;
      if (!(a[i].mode inside {M_A, M_B})) return 0;
      if (a[i].mode == M_B && a[i].level <= 3) return 0;
      if (a[i].zero != 0) return 0;
    end
    return 1;
  endfunction
endclass
module t;
  initial begin
    c o = new();
    int ok = 0, good = 0;
    bit [31:0] first[$];
    for (int k = 0; k < 4; k++) begin
      if (o.randomize()) begin
        ok++;
        good += o.valid();
        first.push_back(o.a[1].offset);
      end
    end
    $display("ok %0d valid %0d size %0d", ok, good, o.a.size());
    $display("varies %0d", first.size() == 4 && !(first[0] == first[1] && first[1] == first[2]));
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["ok 4 valid 4 size 4", "varies 1"]);
}

/// An enum member of a struct element, with no constraint on it, only takes
/// enumerator values, whether the enum is declared locally or in a package.
#[test]
fn enum_members_of_struct_array_elements_stay_in_their_domain() {
    let result = xezim::simulate(
        r#"
package p;
  typedef enum bit [2:0] { P_A = 1, P_B = 5 } pmode_t;
endpackage
typedef enum bit [2:0] { M_A = 1, M_B = 5 } mode_t;
typedef struct { rand mode_t mode; rand bit [3:0] pad; } loc_t;
typedef struct { rand p::pmode_t mode; rand bit [3:0] pad; } pkg_t;
class c;
  rand loc_t l[];
  rand pkg_t g[];
  function new(); l = new[8]; g = new[8]; endfunction
endclass
module t;
  initial begin
    c o = new();
    int bad = 0, ok = 0;
    repeat (20) begin
      ok += o.randomize();
      foreach (o.l[i]) bad += !(o.l[i].mode inside {M_A, M_B});
      foreach (o.g[i]) bad += !(o.g[i].mode inside {p::P_A, p::P_B});
    end
    $display("ok %0d bad %0d", ok, bad);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["ok 20 bad 0"]);
}

/// The same holds for a sub-object solved together with its owner.
#[test]
fn enum_members_of_sub_object_struct_elements_stay_in_their_domain() {
    let result = xezim::simulate(
        r#"
typedef enum bit [2:0] { M_A = 1, M_B = 5 } mode_t;
typedef struct { rand mode_t mode; rand bit [3:0] pad; } loc_t;
typedef class owner_c;
class sub_c;
  owner_c p;
  rand loc_t l[];
  rand bit [3:0] v;
  constraint k { v == p.x; }
  function new(); l = new[8]; endfunction
endclass
class owner_c;
  rand sub_c sub;
  rand bit [3:0] x;
  function new(); sub = new(); sub.p = this; endfunction
endclass
module t;
  initial begin
    owner_c o = new();
    int bad = 0, ok = 0;
    repeat (20) begin
      ok += o.randomize();
      foreach (o.sub.l[i]) bad += !(o.sub.l[i].mode inside {M_A, M_B});
    end
    $display("ok %0d bad %0d", ok, bad);
    $finish;
  end
endmodule
"#,
        100_000,
    )
    .unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|entry| entry.message.as_str())
        .collect();
    assert_eq!(lines, ["ok 20 bad 0"]);
}
