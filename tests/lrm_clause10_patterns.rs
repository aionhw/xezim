//! IEEE 1800-2017 Clause 10 conformance findings — assignment patterns.
//!
//! §10.9.1 array assignment patterns supported only bare positional items. The
//! other three forms all silently wrote element 0 and left the rest at X:
//!   `'{N{expr}}`  replication      -> `'{7, x, x, x}`
//!   `'{default:e}` fill            -> `'{9, x, x, x}`
//!   nested `'{'{1,2},'{3,4}}`      -> untouched (multi-dim arrays weren't found)
//! and a queue pattern dropped replication (`q = '{3{5}}` gave one element).
//!
//! §10.9.2 the `type:value` struct key (`'{byte:3}`) was ignored entirely.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct { byte x; byte y; }   t_t;
  typedef struct { int  a; string s; } p_t;
  typedef struct { int a; real r; }    m_t;

  int  rep  [4];
  int  dflt [4];
  int  keyed[4];
  int  nest [2][2];
  int  desc [3:0];         // descending
  int  d3   [2][2][2];
  t_t  td;
  m_t  mixed;
  p_t  sarr [2];
  real reals [3];
  int  q [$];

  int rep0, rep3, d0, d3v, k0, k1, k2, n01, n10, dd3, dd0, tdx, tdy, q_size, q2;
  int deep;
  int mixed_a;

  initial begin
    rep   = '{4{7}};
    dflt  = '{default:9};
    keyed = '{0:10, 1:20, default:0};
    nest  = '{'{1,2}, '{3,4}};
    desc  = '{1,2,3,4};              // desc[3]=1 .. desc[0]=4
    d3    = '{default:5};            // fill an N-D array
    td    = '{byte:3};               // type-keyed
    mixed = '{a:1, default:0};       // named wins over default
    sarr  = '{'{1,"x"}, '{2,"y"}};   // array of unpacked structs
    reals = '{9.5, 8.25, 7.125};     // real leaves must stay real
    q     = '{3{5}};                 // replication into a queue

    rep0 = rep[0];  rep3 = rep[3];
    d0 = dflt[0];   d3v = dflt[3];
    k0 = keyed[0];  k1 = keyed[1];  k2 = keyed[2];
    n01 = nest[0][1]; n10 = nest[1][0];
    dd3 = desc[3];  dd0 = desc[0];
    deep = d3[1][1][1];
    tdx = td.x;     tdy = td.y;
    mixed_a = mixed.a;
    q_size = q.size(); q2 = q[2];

    $display("SARR=%p", sarr);
    $display("REALS=%p", reals);
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn replication_in_an_array_pattern_fills_every_element() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "rep0"), 7);
    assert_eq!(u(&sim, "rep3"), 7, "replication only wrote element 0");
    // ...and into a queue, where it also sets the size.
    assert_eq!(u(&sim, "q_size"), 3);
    assert_eq!(u(&sim, "q2"), 5);
}

#[test]
fn default_fills_every_remaining_element() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "d0"), 9);
    assert_eq!(u(&sim, "d3v"), 9, "default: only wrote element 0");
    // An N-D array fills through every dimension.
    assert_eq!(u(&sim, "deep"), 5);
    // Index keys coexist with default.
    assert_eq!(u(&sim, "k0"), 10);
    assert_eq!(u(&sim, "k1"), 20);
    assert_eq!(u(&sim, "k2"), 0);
}

#[test]
fn nested_patterns_descend_a_multi_dimensional_array() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "n01"), 2);
    assert_eq!(u(&sim, "n10"), 3);
}

#[test]
fn a_descending_array_binds_positionally_from_its_left_index() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "dd3"), 1, "desc[3] takes the first item");
    assert_eq!(u(&sim, "dd0"), 4);
}

#[test]
fn type_keyed_struct_pattern_binds_members_of_that_type() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "tdx"), 3, "the type-keyed pattern was ignored");
    assert_eq!(u(&sim, "tdy"), 3);
    // A `name:` key still wins over `default:`.
    assert_eq!(u(&sim, "mixed_a"), 1);
}

#[test]
fn struct_and_real_elements_survive_the_rewritten_array_path() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        line(&sim, "SARR="),
        r#"SARR='{'{a:1, s:"x"}, '{a:2, s:"y"}}"#
    );
    // A real leaf must not be truncated to an integer.
    assert_eq!(line(&sim, "REALS="), "REALS='{9.5, 8.25, 7.125}");
}
