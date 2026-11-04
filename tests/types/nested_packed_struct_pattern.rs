//! §10.9.2: a named assignment pattern onto a packed struct whose member is
//! itself a packed struct.
//!
//! The field layout is flattened, so the nested member is recorded twice:
//! once as the parent (`exception`) and again as each leaf (`exception.cause`).
//! The compiled pattern concatenated every row. The value was wider than the
//! struct, the assignment kept the low bits, and a named sibling such as
//! `trans_id` landed outside its slot. A later `resp.result = …` repaired the
//! fields it wrote and left `trans_id` at 0. The same pattern in an `initial`
//! block, which stays on the interpreter, was already correct.
//!
//! `always_comb` compiles. Both a local and a type-parameter port must come
//! back with the id in its own field.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

const SRC: &str = r#"
typedef struct packed {
  logic [7:0] hi;
  logic [3:0] mid;
  logic [3:0] lo;
} flat_t;

typedef struct packed {
  logic [7:0] cause;
  logic       valid;
} exc_t;

typedef struct packed {
  logic       req_ready;
  logic       resp_valid;
  logic [7:0] result;
  logic [3:0] trans_id;
  exc_t       exception;
  logic       fflags_valid;
} nest_t;

module leaf #(
  parameter type T = logic
) (
  input  logic [3:0] id_i,
  input  logic [7:0] data_i,
  output T           resp_o
);
  always_comb begin
    resp_o = '{
      trans_id    : id_i,
      fflags_valid: 1'b1,
      default     : '0
    };
    resp_o.resp_valid = 1'b1;
    resp_o.result     = data_i;
  end
endmodule

module tb;
  logic [3:0] id;
  logic [7:0] data;
  nest_t resp, local_n, init_n, nested_pat;
  flat_t flat;
  int p_tid, p_res, p_rv, p_ff;
  int l_tid, l_res, l_ff;
  int i_tid, i_res;
  int n_tid, n_cause, n_valid, n_res;
  int f_hi, f_lo;

  leaf #(.T(nest_t)) u (.id_i(id), .data_i(data), .resp_o(resp));

  // Separate blocks: one statement that fails to compile bails the
  // whole always_comb onto the interpreter, which already packed this
  // correctly. The local and the port must stay on the compiled path.
  always_comb begin
    flat = '{hi: data, lo: id, default: '0};
    local_n = '{
      trans_id    : id,
      fflags_valid: 1'b1,
      default     : '0
    };
    local_n.resp_valid = 1'b1;
    local_n.result     = data;
  end
  always_comb begin
    nested_pat = '{
      exception   : '{cause: 8'hAB, valid: 1'b1},
      trans_id    : id,
      result      : data,
      resp_valid  : 1'b1,
      default     : '0
    };
  end

  initial begin
    id   = 4'h1;
    data = 8'h04;
    #1;
    init_n = '{
      trans_id    : id,
      fflags_valid: 1'b1,
      default     : '0
    };
    init_n.resp_valid = 1'b1;
    init_n.result     = data;
    p_tid = resp.trans_id;     p_res = resp.result;
    p_rv  = resp.resp_valid;   p_ff  = resp.fflags_valid;
    l_tid = local_n.trans_id;  l_res = local_n.result;  l_ff = local_n.fflags_valid;
    i_tid = init_n.trans_id;   i_res = init_n.result;
    n_tid = nested_pat.trans_id; n_cause = nested_pat.exception.cause;
    n_valid = nested_pat.exception.valid; n_res = nested_pat.result;
    f_hi = flat.hi; f_lo = flat.lo;
  end
endmodule
"#;

#[test]
fn nested_member_pattern_keeps_sibling_fields() {
    let sim = simulate(SRC, 50).expect("simulate failed");
    assert_eq!(u(&sim, "f_hi"), 0x04, "flat pattern still packs hi");
    assert_eq!(u(&sim, "f_lo"), 0x1, "flat pattern still packs lo");
    assert_eq!(u(&sim, "p_tid"), 1, "type-parameter port keeps trans_id");
    assert_eq!(u(&sim, "p_res"), 0x04, "port result write still lands");
    assert_eq!(u(&sim, "p_rv"), 1, "port resp_valid write still lands");
    assert_eq!(u(&sim, "p_ff"), 1, "port fflags_valid from the pattern");
    assert_eq!(u(&sim, "l_tid"), 1, "local always_comb keeps trans_id");
    assert_eq!(u(&sim, "l_res"), 0x04);
    assert_eq!(u(&sim, "l_ff"), 1);
    assert_eq!(
        u(&sim, "i_tid"),
        1,
        "initial-block pattern was already correct"
    );
    assert_eq!(u(&sim, "i_res"), 0x04);
    assert_eq!(u(&sim, "n_tid"), 1, "nested pattern keeps the sibling id");
    assert_eq!(
        u(&sim, "n_cause"),
        0xAB,
        "nested pattern fills the inner member"
    );
    assert_eq!(u(&sim, "n_valid"), 1);
    assert_eq!(u(&sim, "n_res"), 0x04);
}
