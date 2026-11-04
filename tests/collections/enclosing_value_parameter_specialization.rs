//! §8.25: a specialization written inside a parameterized class may name
//! that class's value parameters. `item #(H, D) req = new;` inside
//! `my_seq #(32, 194)` builds `item #(32, 194)`, and the `RSP` type derived
//! from `item#(H, D)` in an `extends` clause is that same specialization. The
//! bare names used to fall back to `item`'s defaults, so `put_response`
//! rejected its own request.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{n}")))
        .unwrap_or_else(|| panic!("signal not found: {n}"))
        .to_u64()
        .unwrap_or_else(|| panic!("{n} not u64-able"))
        & 0xFFFF_FFFF
}

#[test]
fn new_inside_a_method_uses_the_enclosing_value_parameters() {
    let src = r#"
class base_item;
endclass

class item #(int H = 32, int D = H) extends base_item;
  int tag;
endclass

class uvm_sequence #(type REQ = base_item, type RSP = REQ);
  virtual function int put_response(base_item response_item);
    RSP response;
    return $cast(response, response_item);
  endfunction
endclass

class mid #(type REQ = base_item, type RSP = REQ) extends uvm_sequence #(REQ, RSP);
endclass

class my_seq #(int H = 32, int D = H) extends mid #(item #(H, D), item #(H, D));
  function int check();
    item #(H, D) req;
    req = new;
    req.tag = D;
    return put_response(req);
  endfunction
  function int seen_d();
    item #(H, D) req;
    req = new;
    return req.D;
  endfunction
  function int check_lit();
    item #(32, 194) req;
    req = new;
    return put_response(req);
  endfunction
endclass

module top;
  my_seq #(32, 194) wide;
  my_seq #(32, 32) narrow;
  item #(32, 32) nitem;
  item #(32, 194) witem;
  int wide_ok, narrow_ok, swapped, explicit, lit_ok, got_d;
  initial begin
    wide = new;
    narrow = new;
    nitem = new;
    witem = new;
    got_d = wide.seen_d();
    wide_ok = wide.check();
    narrow_ok = narrow.check();
    swapped = wide.put_response(nitem);
    explicit = wide.put_response(witem);
    lit_ok = wide.check_lit();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = (
        u(&sim, "wide_ok"),
        u(&sim, "narrow_ok"),
        u(&sim, "swapped"),
        u(&sim, "explicit"),
        u(&sim, "lit_ok"),
        u(&sim, "got_d"),
    );
    assert_eq!(
        got,
        (1, 1, 0, 1, 1, 194),
        "wide, narrow, swapped, explicit, lit, got_d"
    );
}

/// `dv_base_seq#(.REQ(item#(H, D)))` leaves RSP at its default, which is
/// REQ. `item#(H, D)` must stay this sequence's H and D. Those names are
/// also item's own parameters, so the default substitution used to read
/// them as omitted arguments and bind RSP to `item#(32, 32)`. put_response
/// then rejected an `item#(32, 192)`.
#[test]
fn defaulted_rsp_keeps_the_outer_value_parameters() {
    let src = r#"
class uvm_sequence_item;
endclass
class uvm_object;
endclass
class uvm_sequencer;
endclass

class uvm_sequence #(type REQ = uvm_sequence_item, type RSP = REQ);
  virtual function int put_response(uvm_sequence_item response_item);
    RSP response;
    return $cast(response, response_item);
  endfunction
endclass

class item #(int H = 32, int D = H) extends uvm_sequence_item;
  int tag;
  function new();
    tag = D;
  endfunction
endclass

class dv_base_seq #(type REQ = uvm_sequence_item,
                    type RSP = REQ,
                    type CFG_T = uvm_object,
                    type SEQUENCER_T = uvm_sequencer)
  extends uvm_sequence#(REQ, RSP);
endclass

class base_seq #(int H = 32, int D = H)
  extends dv_base_seq#(
    .REQ(item#(H, D)),
    .CFG_T(uvm_object),
    .SEQUENCER_T(uvm_sequencer)
  );
endclass

class dev_seq #(int H = 32, int D = H) extends base_seq#(H, D);
  function int go();
    item#(H, D) req;
    req = new;
    return put_response(req);
  endfunction
  function int seen_d();
    item#(H, D) req;
    req = new;
    return req.tag;
  endfunction
  function int narrow();
    item#(.H(7)) req;
    req = new;
    return req.tag;
  endfunction
endclass

module top;
  dev_seq#(32, 192) s;
  int ok, got_d, got_narrow;
  initial begin
    s = new;
    got_d = s.seen_d();
    ok = s.go();
    got_narrow = s.narrow();
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(
        (u(&sim, "got_d"), u(&sim, "ok"), u(&sim, "got_narrow")),
        (192, 1, 7),
        "item width, put_response cast, and an omitted width stays the default"
    );
}

/// `child #(N)` built inside `outer #(194)` passes N on to its parent.
#[test]
fn new_passes_an_enclosing_value_parameter_to_an_ancestor() {
    let src = r#"
class parent #(int P = 8);
  int pw;
  function new(); pw = P; endfunction
endclass
class child #(int W = 32) extends parent #(W);
endclass
class outer #(int N = 32);
  function int go(); child #(N) c; c = new; return c.pw; endfunction
endclass
module top;
  outer #(194) o;
  int got;
  initial begin o = new; got = o.go(); end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "got"), 194);
}
