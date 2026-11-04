//! Bugs found debugging a hierarchical-path split (a `ref string q[$]` helper
//! that indexes a string char by char). Six distinct defects, all silent.
//!
//! 1. §13.5.2 — a `ref`/`output`/`inout` QUEUE argument was never written back:
//!    `push_back` inside the callee mutated a copy, and the caller's queue was
//!    unchanged on return. (Arrays and assoc arrays already wrote back.)
//! 2. §11.4.13 — `s[i]` on a string VARIABLE returned 0. A string is stored in
//!    a fixed-width container, so `width/8` is the container size, not the
//!    string length; indexing that read the empty leading bytes.
//! 3. `s[i] = c` wrote to the wrong byte and grew the string.
//! 4. §21.2.1.7 — `%p` on a LOCAL queue printed `x` (only module-scope queues
//!    rendered).
//! 5. A `string q[$]` was not marked as string-valued, so its elements printed
//!    as integers instead of quoted strings.
//! 6. `q = {}` did not clear a string queue (it took the byte-concat path and
//!    left one empty element).

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 10_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_ref_queue_argument_is_written_back() {
    let o = out(r#"
package p;
  function automatic void f(ref int q[$]); q.push_back(1); q.push_back(2); endfunction
  function automatic void g(ref string q[$]); q.push_back("a"); q.push_back("b"); endfunction
endpackage
module m; initial begin
  int qi[$]; string qs[$];
  p::f(qi); p::g(qs);
  $display("QI=%0d,%0d,%0d", qi.size(), qi[0], qi[1]);
  $display("QS=%0d,%s,%s", qs.size(), qs[0], qs[1]);
end endmodule
"#);
    assert!(
        o.contains("QI=2,1,2"),
        "ref int queue not written back: {}",
        o
    );
    assert!(
        o.contains("QS=2,a,b"),
        "ref string queue not written back: {}",
        o
    );
}

#[test]
fn string_variable_indexing_reads_the_character() {
    let o = out(r#"
module m; initial begin
  string s = "abc";       // local
  $display("L=%0d,%0d,%0d", s[0], s[1], s[2]);
end endmodule
"#);
    // 'a'=97 'b'=98 'c'=99
    assert!(o.contains("L=97,98,99"), "string[i] read wrong: {}", o);
}

#[test]
fn a_string_variable_passed_to_a_param_indexes_correctly() {
    let o = out(r#"
package p; function automatic int at(string s, int i); return s[i]; endfunction endpackage
module m; initial begin
  string t = "xy";
  $display("V=%0d L=%0d", p::at(t, 1), p::at("xy", 1));
end endmodule
"#);
    assert!(
        o.contains("V=121 L=121"),
        "passing a string var lost its bytes: {}",
        o
    );
}

#[test]
fn string_variable_index_assignment_replaces_the_character() {
    let o = out(r#"
module m; initial begin
  string s = "abc";
  s[1] = "Z";
  $display("W=%s", s);
end endmodule
"#);
    assert!(
        o.contains("W=aZc"),
        "string[i]= wrote the wrong char: {}",
        o
    );
}

#[test]
fn percent_p_on_a_local_string_queue_renders_elements() {
    let o = out(r#"
module m; initial begin
  string q[$]; int n[$];
  q.push_back("a"); q.push_back("bb");
  n.push_back(1); n.push_back(2);
  $display("SQ=%p", q);
  $display("NQ=%p", n);
end endmodule
"#);
    assert!(
        o.contains(r#"SQ='{"a", "bb"}"#),
        "string queue %p wrong: {}",
        o
    );
    assert!(o.contains("NQ='{1, 2}"), "int queue %p wrong: {}", o);
}

#[test]
fn clearing_a_string_queue_empties_it() {
    let o = out(r#"
module m; initial begin
  string q[$];
  q.push_back("a"); q.push_back("b");
  q = {};
  $display("C=%0d %p", q.size(), q);
end endmodule
"#);
    assert!(
        o.contains("C=0 '{}"),
        "q = {{}} did not clear the queue: {}",
        o
    );
}

#[test]
fn percent_p_on_a_local_string_valued_assoc_quotes_the_values() {
    // §21.2.1.7 — a LOCAL `string m[...]` (string-VALUED associative array) must
    // render its values quoted, not as their character codes. Covers both an
    // int-keyed and a string-keyed map. (Module-scope arrays already worked via
    // `var_decl_types`; a local has no such entry, so the name must be marked.)
    let o = out(r#"
module m; initial begin
  string ai[int]; string sk[string];
  ai[0] = "x"; ai[1] = "y";
  sk["a"] = "apple";
  $display("AI=%p", ai);
  $display("SK=%p", sk);
  // reads must be unaffected by the string marking
  $display("R=%s L=%0d", ai[1], ai[1].len());
end endmodule
"#);
    assert!(
        o.contains(r#"AI='{0:"x", 1:"y"}"#),
        "int-keyed string assoc %p: {}",
        o
    );
    assert!(
        o.contains(r#"SK='{"a":"apple"}"#),
        "string-keyed string assoc %p: {}",
        o
    );
    assert!(
        o.contains("R=y L=1"),
        "string-valued assoc read broke: {}",
        o
    );
}

/// The whole helper end to end: split on '.' then on ':', with a clear between.
#[test]
fn the_hierarchical_split_helper_works_end_to_end() {
    let o = out(r#"
package cup;
  string dummycomp [$];
  byte cref_delim = ".";
  function automatic void split(string s1, ref string components[$] = dummycomp, const ref byte delim = cref_delim);
    int last_sep_idx = -1, curr = 0;
    for (int i = 0; i < s1.len(); i++) begin
      if ((s1[i] == delim) || (i == (s1.len()-1))) begin
        components.push_back(s1.substr(last_sep_idx + 1, last_sep_idx + curr + (s1[i] != delim)));
        last_sep_idx = i; curr = 0;
      end else curr++;
    end
  endfunction
endpackage
module tb; initial begin
  string myc[$]; byte delim;
  cup::split("a.bb.ccc", myc);
  $display("DOT=%p", myc);
  myc = {};
  delim = ":";
  cup::split("x:yy:zzz", myc, delim);
  $display("COLON=%p", myc);
end endmodule
"#);
    assert!(o.contains(r#"DOT='{"a", "bb", "ccc"}"#), "dot split: {}", o);
    assert!(
        o.contains(r#"COLON='{"x", "yy", "zzz"}"#),
        "colon split: {}",
        o
    );
}
