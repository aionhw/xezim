`timescale 1ns/1ps
// IEEE 1800-2023 §7.4.2, §10.9.1 / §10.9.2: declared indices,
// index keys, defaults and replication; §10.3.2: reevaluate on x changes.
// Sample one tick after stimulus (§4.7); no declaration initializers (§6.8).
`define C(n, g, e) if ((g) === (e)) $display("PASS %s", n); else $display("FAIL %s got=%0d expected=%0d", n, g, e);
typedef struct { int tag; int a[-1:1]; } Asc;
typedef struct { int tag; int a[1:-1]; } Desc;
typedef struct { int tag; int a[-3:-1]; } NegAsc;
typedef struct { int tag; int a[-1:-3]; } NegDesc;
typedef struct { int tag; int a[0:2]; } Pos;
typedef struct { int tag; int a[-1:0][1:-1]; } Matrix;
typedef struct { int p; int q; } Pair;
typedef struct { int tag; Pair a[-1:0]; } Pairs;
// §6.6.7 / §10.3.3: whole-net assignments to unresolved user nettypes.
nettype Asc AscNet;
nettype Desc DescNet;
// §6.6.7: a resolver is a pure function of whole-structure drivers.
function automatic Asc resolve_asc(input Asc drivers[]);
  Asc result;
  result = '{default:0};
  foreach (drivers[i]) begin
    result.tag += drivers[i].tag;
    result.a[-1] += drivers[i].a[-1];
    result.a[0] += drivers[i].a[0];
    result.a[1] += drivers[i].a[1];
  end
  return result;
endfunction
nettype Asc ResolvedNet with resolve_asc;
// §23.3.3: a struct input actual supplied by a contextual assignment pattern.
module sink(input Asc s, output Asc o);
  assign o = s;
endmodule
module child(input int x, output Asc o);
  assign o = '{tag:x, a:'{-1:x+8, default:x+16}};
endmodule
module top;
  int x;
  Asc keys, defaults, key_first, default_first, rep, ordered, copied, array_item;
  Desc desc_keys, desc_rep;
  NegAsc neg_keys, neg_default;
  NegDesc neg_desc, neg_rep;
  Pos positive;
  Matrix matrix;
  Pairs pairs;
  AscNet net_keys;
  DescNet net_default;
  ResolvedNet resolved_keys, resolved_default, resolved_copy;
  Asc child_o, port_o, generated, generated_child;
  Asc procedural;
  int whole[-1:1], source[-1:1];
  assign keys = '{tag:x, a:'{1:x+10, -1:x+8, 0:x+9}};
  assign defaults = '{tag:x, a:'{default:x+16}};
  assign key_first = '{tag:x, a:'{0:x+9, default:x+16}};
  assign default_first = '{tag:x, a:'{default:x+16, -1:x+8}};
  assign rep = '{tag:x, a:'{3{x+11}}};
  assign ordered = '{tag:x, a:'{x+8, x+9, x+10}};
  assign desc_keys = '{tag:x, a:'{-1:x+8, 0:x+9, 1:x+10}};
  assign desc_rep = '{tag:x, a:'{1{x+8, x+9, x+10}}};
  assign neg_keys = '{tag:x, a:'{-3:x+8, -1:x+10, -2:x+9}};
  assign neg_default = '{tag:x, a:'{default:x+16, -2:x+9}};
  assign neg_desc = '{tag:x, a:'{-2:x+9, default:x+16}};
  assign neg_rep = '{tag:x, a:'{1{x+8, x+9, x+10}}};
  assign positive = '{tag:x, a:'{0:x+8, default:x+16}};
  assign matrix = '{tag:x, a:'{-1:'{1:x+8, default:x+16}, 0:'{3{x+9}}}};
  assign pairs = '{tag:x, a:'{-1:'{p:x+8, q:x+9}, default:Pair'{p:x+16, q:x+17}}};
  assign net_keys = '{tag:x, a:'{-1:x+8, 0:x+9, 1:x+10}};
  assign net_default = '{tag:x, a:'{default:x+16, 0:x+9}};
  assign resolved_keys = '{tag:x, a:'{-1:x+8, 0:x+9, 1:x+10}};
  assign resolved_default = '{tag:x, a:'{0:x+9, default:x+16}};
  assign copied = ordered;
  assign resolved_copy = ordered;
  assign source = '{-1:x+8, 0:x+9, 1:x+10};
  assign whole = '{-1:x+8, 0:x+9, 1:x+10};
  assign array_item = '{tag:x, a:source};
  child u_child(.x(x), .o(child_o));
  sink u_sink(.s('{tag:x, a:'{-1:x+8, 0:x+9, 1:x+10}}), .o(port_o));
  // §27.5: the same pattern in a named generate scope.
  if (1) begin : g
    assign generated = '{tag:x, a:'{-1:x+8, default:x+16}};
    child u(.x(x), .o(generated_child));
  end
  task automatic sample();
    `C("keys.a[-1]", keys.a[-1], x+8)
    `C("keys.a[0]", keys.a[0], x+9)
    `C("keys.a[1]", keys.a[1], x+10)
    `C("keys.tag", keys.tag, x)
    `C("defaults.a[-1]", defaults.a[-1], x+16)
    `C("defaults.a[0]", defaults.a[0], x+16)
    `C("defaults.a[1]", defaults.a[1], x+16)
    `C("defaults.tag", defaults.tag, x)
    `C("key_first.a[-1]", key_first.a[-1], x+16)
    `C("key_first.a[0]", key_first.a[0], x+9)
    `C("key_first.a[1]", key_first.a[1], x+16)
    `C("key_first.tag", key_first.tag, x)
    `C("default_first.a[-1]", default_first.a[-1], x+8)
    `C("default_first.a[0]", default_first.a[0], x+16)
    `C("default_first.a[1]", default_first.a[1], x+16)
    `C("default_first.tag", default_first.tag, x)
    `C("rep.a[-1]", rep.a[-1], x+11)
    `C("rep.a[0]", rep.a[0], x+11)
    `C("rep.a[1]", rep.a[1], x+11)
    `C("rep.tag", rep.tag, x)
    `C("ordered.a[-1]", ordered.a[-1], x+8)
    `C("ordered.a[0]", ordered.a[0], x+9)
    `C("ordered.a[1]", ordered.a[1], x+10)
    `C("ordered.tag", ordered.tag, x)
    `C("desc_keys.a[-1]", desc_keys.a[-1], x+8)
    `C("desc_keys.a[0]", desc_keys.a[0], x+9)
    `C("desc_keys.a[1]", desc_keys.a[1], x+10)
    `C("desc_keys.tag", desc_keys.tag, x)
    `C("desc_rep.a[1]", desc_rep.a[1], x+8)
    `C("desc_rep.a[0]", desc_rep.a[0], x+9)
    `C("desc_rep.a[-1]", desc_rep.a[-1], x+10)
    `C("desc_rep.tag", desc_rep.tag, x)
    `C("neg_keys.a[-3]", neg_keys.a[-3], x+8)
    `C("neg_keys.a[-2]", neg_keys.a[-2], x+9)
    `C("neg_keys.a[-1]", neg_keys.a[-1], x+10)
    `C("neg_keys.tag", neg_keys.tag, x)
    `C("neg_default.a[-3]", neg_default.a[-3], x+16)
    `C("neg_default.a[-2]", neg_default.a[-2], x+9)
    `C("neg_default.a[-1]", neg_default.a[-1], x+16)
    `C("neg_default.tag", neg_default.tag, x)
    `C("neg_desc.a[-3]", neg_desc.a[-3], x+16)
    `C("neg_desc.a[-2]", neg_desc.a[-2], x+9)
    `C("neg_desc.a[-1]", neg_desc.a[-1], x+16)
    `C("neg_desc.tag", neg_desc.tag, x)
    `C("neg_rep.a[-1]", neg_rep.a[-1], x+8)
    `C("neg_rep.a[-2]", neg_rep.a[-2], x+9)
    `C("neg_rep.a[-3]", neg_rep.a[-3], x+10)
    `C("neg_rep.tag", neg_rep.tag, x)
    `C("positive.a[0]", positive.a[0], x+8)
    `C("positive.a[1]", positive.a[1], x+16)
    `C("positive.a[2]", positive.a[2], x+16)
    `C("positive.tag", positive.tag, x)
    `C("net_keys.a[-1]", net_keys.a[-1], x+8)
    `C("net_keys.a[0]", net_keys.a[0], x+9)
    `C("net_keys.a[1]", net_keys.a[1], x+10)
    `C("net_keys.tag", net_keys.tag, x)
    `C("net_default.a[-1]", net_default.a[-1], x+16)
    `C("net_default.a[0]", net_default.a[0], x+9)
    `C("net_default.a[1]", net_default.a[1], x+16)
    `C("net_default.tag", net_default.tag, x)
    `C("resolved_keys.a[-1]", resolved_keys.a[-1], x+8)
    `C("resolved_keys.a[0]", resolved_keys.a[0], x+9)
    `C("resolved_keys.a[1]", resolved_keys.a[1], x+10)
    `C("resolved_keys.tag", resolved_keys.tag, x)
    `C("resolved_default.a[-1]", resolved_default.a[-1], x+16)
    `C("resolved_default.a[0]", resolved_default.a[0], x+9)
    `C("resolved_default.a[1]", resolved_default.a[1], x+16)
    `C("resolved_default.tag", resolved_default.tag, x)
    `C("resolved_copy.a[-1]", resolved_copy.a[-1], x+8)
    `C("resolved_copy.a[0]", resolved_copy.a[0], x+9)
    `C("resolved_copy.a[1]", resolved_copy.a[1], x+10)
    `C("resolved_copy.tag", resolved_copy.tag, x)
    `C("copied.a[-1]", copied.a[-1], x+8)
    `C("copied.a[0]", copied.a[0], x+9)
    `C("copied.a[1]", copied.a[1], x+10)
    `C("copied.tag", copied.tag, x)
    `C("array_item.a[-1]", array_item.a[-1], x+8)
    `C("array_item.a[0]", array_item.a[0], x+9)
    `C("array_item.a[1]", array_item.a[1], x+10)
    `C("array_item.tag", array_item.tag, x)
    `C("child_o.a[-1]", child_o.a[-1], x+8)
    `C("child_o.a[0]", child_o.a[0], x+16)
    `C("child_o.a[1]", child_o.a[1], x+16)
    `C("child_o.tag", child_o.tag, x)
    `C("port_o.a[-1]", port_o.a[-1], x+8)
    `C("port_o.a[0]", port_o.a[0], x+9)
    `C("port_o.a[1]", port_o.a[1], x+10)
    `C("port_o.tag", port_o.tag, x)
    `C("generated.a[-1]", generated.a[-1], x+8)
    `C("generated.a[0]", generated.a[0], x+16)
    `C("generated.a[1]", generated.a[1], x+16)
    `C("generated.tag", generated.tag, x)
    `C("generated_child.a[-1]", generated_child.a[-1], x+8)
    `C("generated_child.a[0]", generated_child.a[0], x+16)
    `C("generated_child.a[1]", generated_child.a[1], x+16)
    `C("generated_child.tag", generated_child.tag, x)
    `C("procedural.a[-1]", procedural.a[-1], x+8)
    `C("procedural.a[0]", procedural.a[0], x+9)
    `C("procedural.a[1]", procedural.a[1], x+10)
    `C("procedural.tag", procedural.tag, x)
    `C("matrix[-1][1]", matrix.a[-1][1], x+8)
    `C("matrix[-1][0]", matrix.a[-1][0], x+16)
    `C("matrix[-1][-1]", matrix.a[-1][-1], x+16)
    `C("matrix[0][1]", matrix.a[0][1], x+9)
    `C("matrix[0][0]", matrix.a[0][0], x+9)
    `C("matrix[0][-1]", matrix.a[0][-1], x+9)
    `C("matrix.tag", matrix.tag, x)
    `C("pairs[-1].p", pairs.a[-1].p, x+8)
    `C("pairs[-1].q", pairs.a[-1].q, x+9)
    `C("pairs[0].p", pairs.a[0].p, x+16)
    `C("pairs[0].q", pairs.a[0].q, x+17)
    `C("pairs.tag", pairs.tag, x)
    `C("whole[-1]", whole[-1], x+8)
    `C("whole[0]", whole[0], x+9)
    `C("whole[1]", whole[1], x+10)
  endtask
  initial begin
    #1 x = 7;
    procedural = '{tag:x, a:'{-1:x+8, 0:x+9, 1:x+10}};
    #1 sample();
    #1 x = -5;
    procedural = '{tag:x, a:'{-1:x+8, 0:x+9, 1:x+10}};
    #1 sample();
    $finish;
  end
endmodule
`undef C
