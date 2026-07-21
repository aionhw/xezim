// Regression tests for instance-collection (associative array / queue /
// dynamic array) properties reached through an ARRAY-ELEMENT base —
// `arr[i].coll_field` and `foreach (arr[i].coll_field[k])`.
//
// Root cause: xezim stored instance collection properties as signals
// `<handle>#field[key]` (NOT in the heap object's `properties` map), and
// `expr_assoc_name` only resolved the collection for a bare `Ident`/`this`
// base. An array-element base (`arr[i]`, flattened by the parser to a
// 2-segment `Ident([arr[selects=[i]], field])`, or a `MemberAccess{Index}`)
// fell through, so:
//   - `arr[i].aa.num()` returned 0,
//   - `foreach (arr[i].aa[k])` iterated the WRONG object's collection
//     (`this.aa` instead of `arr[i].aa`).
//
// This silently corrupted any graph stored as `obj[]` of handles each
// carrying assoc-array edges — most notably UVM's phase graph, where
// `foreach (successors[s].m_predecessors[pred])` iterated `this`'s
// (the caller's) predecessors instead of `successors[s]`'s, so the phase
// sibling graph read the wrong object's edges and `common.run` ended
// without waiting for the runtime phase schedule.
//
// Verified against the standalone semantics (handle-keyed AA stored under
// `<handle>#field`, distinct per instance).

use std::process::Command;

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_arrcoll_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}.sv"));
    std::fs::write(&path, src).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let out = Command::new(bin)
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(path.to_str().unwrap())
        .output()
        .expect("failed to run xezim");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// `arr[i].aa.num()` and `foreach (arr[i].aa[k])` must resolve to the
// i-th element's collection, not `this`'s. Each element gets distinct keys.
#[test]
fn array_element_assoc_num_and_foreach() {
    let src = r#"class C; int id; function new(int i); id=i; endfunction endclass
typedef bit edges_t[C];
class Node;
  string name; edges_t m_preds;
  function new(string n); name=n; endfunction
  function void addp(C k); m_preds[k]=1; endfunction
endclass
module top;
  initial begin
    Node arr[] = new[2];
    Node a=new("a"), b=new("b");
    C k1=new(101), k2=new(202), k3=new(303);
    arr[0]=a; arr[1]=b;
    arr[0].addp(k1); arr[0].addp(k2);   // a.m_preds = {k1,k2}
    arr[1].addp(k3);                      // b.m_preds = {k3}
    // .num() via array-element base
    if (arr[0].m_preds.num() != 2) $display("FAIL a.num=%0d", arr[0].m_preds.num());
    else if (arr[1].m_preds.num() != 1) $display("FAIL b.num=%0d", arr[1].m_preds.num());
    else begin
      // foreach via array-element base (UVM phase-graph pattern)
      foreach (arr[i]) begin
        foreach (arr[i].m_preds[p])
          $display("elem %s key id=%0d", arr[i].name, p.id);
      end
      $display("PASS array-element-assoc");
    end
  end
endmodule
"#;
    let out = run(src, "arr_assoc");
    assert!(
        out.contains("PASS array-element-assoc") && !out.contains("FAIL"),
        "array-element assoc resolution failed.\n{out}"
    );
    // Each element's keys must be distinct and correct.
    assert!(out.contains("elem a key id=101"), "missing a/k1: {out}");
    assert!(out.contains("elem a key id=202"), "missing a/k2: {out}");
    assert!(out.contains("elem b key id=303"), "missing b/k3: {out}");
}

// The UVM phase-graph pattern distilled: a dynamic array of "node" handles,
// each carrying an assoc-array of edges (predecessor handles). Building the
// graph via array-element writes, then traversing via array-element foreach
// must see each node's OWN edges — not the traversing object's.
#[test]
fn array_element_graph_traversal() {
    let src = r#"class C; int id; function new(int i); id=i; endfunction endclass
typedef bit edges_t[C];
class Node;
  int id; edges_t m_succs;
  function new(int i); id=i; endfunction
  function void link(Node s); m_succs[s]=1; endfunction
endclass
module top;
  initial begin
    Node n1=new(1), n2=new(2), n3=new(3), n4=new(4);
    Node nodes[] = new[4];
    nodes[0]=n1; nodes[1]=n2; nodes[2]=n3; nodes[3]=n4;
    // 1->2, 2->3, 3->4  (chain via array-element writes)
    nodes[0].link(nodes[1]);
    nodes[1].link(nodes[2]);
    nodes[2].link(nodes[3]);
    // traverse: for each node, foreach its successors
    int total_edges = 0;
    foreach (nodes[n]) begin
      foreach (nodes[n].m_succs[s]) begin
        total_edges++;
        $display("edge %0d -> %0d", nodes[n].id, s.id);
      end
    end
    if (total_edges == 3) $display("PASS graph-traversal");
    else $display("FAIL total_edges=%0d (expected 3)", total_edges);
  end
endmodule
"#;
    let out = run(src, "graph_trav");
    assert!(
        out.contains("PASS graph-traversal") && !out.contains("FAIL"),
        "array-element graph traversal failed.\n{out}"
    );
}
