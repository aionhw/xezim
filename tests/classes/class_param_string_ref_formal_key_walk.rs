//! §7.9.4 first/next on an associative array whose KEY is a type parameter
//! bound to `string` (`class Pool #(type KEY=string); int aa[KEY]; ... `):
//! the KEY type isn't resolved to `SimpleType::String` at elaboration, so
//! `assoc_index_width_for` reported a spurious 1024-bit index width. The
//! `first`/`next` builtin then took the WIDE-NUMERIC hex-key branch and
//! shipped the key's ASCII bytes as an integer into the ref formal
//! ("ID103" -> 2 bytes of garbage) instead of writing the string key. UVM's
//! report handler iterates its `sep_threshold_id_file_handles` pools exactly
//! this way (with `ref KEY key`), so a multi-byte key iterated through
//! `first`/`next` truncated and the pool walk spun forever.
//!
//! Every expectation below is byte-for-byte identical to the reference
//! simulator: the ref formal must receive the full-width string key in the
//! same lexicographic order Questa produces.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 200)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

const SRC: &str = r#"
class Pool #(type KEY=string, type VAL=int);
  VAL aa[KEY];
  function void add(KEY k, VAL v_); aa[k] = v_; endfunction
  function int first(ref KEY key); return aa.first(key); endfunction
  function int next(ref KEY key); return aa.next(key); endfunction
endclass
class User;
  Pool#(string,int) p;
  string idx;
  function new(); p = new; endfunction
  function void run();
    int f;
    int cnt;
    f = p.first(idx);
    $display("U first='%s' f=%0d len=%0d", idx, f, idx.len());
    cnt = 0;
    while (p.next(idx)) begin
      $display("U next='%s' len=%0d", idx, idx.len());
      cnt++;
    end
    $display("U count=%0d", cnt);
  endfunction
endclass
module top;
  initial begin
    User u = new;
    u.p.add("ID207", 5);
    u.p.add("ID103", 6);
    u.p.add("ID7", 7);
    u.run();
  end
endmodule
"#;

#[test]
fn string_key_via_type_param_first_next_writes_full_width() {
    let got: Vec<String> = msgs(SRC)
        .into_iter()
        .filter(|l| !l.starts_with("Simulation"))
        .collect();
    let want = [
        "U first='ID103' f=1 len=5",
        "U next='ID207' len=5",
        "U next='ID7' len=3",
        "U count=2",
    ];
    assert_eq!(got, want.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{got:?}");
}