//! IEEE 1800-2023 §7.2, §8.11 and §26.3: aggregate storage follows
//! its declaring package or object, including selected handle receivers.
use xezim::simulate;

#[test]
fn package_arrays_keep_distinct_cells_and_bounds() {
    let src = r#"
package p;
 int a[-1:1];
 function int read_a(); return a[1]; endfunction
endpackage
package q;
 int a[2:3];
endpackage
module imported;
 import p::*;
 initial begin #2; $display("import=%0d", a[1]); end
endmodule
module top;
 int a[0:1];
 int observed;
 assign observed=a[1];
 imported u();
 initial begin
  #1 a[1]=6; p::a[1]=38; p::a[-1]=9; q::a[3]=47;
  #2 $display("values=%0d %0d %0d %0d %0d",a[1],observed,p::a[1],q::a[3],p::read_a());
  $display("bounds=%0d %0d",$low(p::a),$high(q::a));
 end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulation");

    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("values=6 6 38 47 38"), "{out}");
    assert!(out.contains("import=38"), "{out}");
    assert!(out.contains("bounds=-1 3"), "{out}");
}

#[test]
fn selected_class_receivers_keep_object_storage() {
    let src = r#"
module top;
 typedef enum bit[1:0] {L0,L1,L2,L3} level_e;
 typedef struct packed {bit[3:0] code; level_e lvl;} item_t;
 class node;
  level_e arr[2]; item_t sarr[2];
  function new(); arr[0]=L1; sarr[1].lvl=L3; endfunction
  function string name_value(); return this.sarr[1].lvl.name(); endfunction
 endclass
 item_t sarr[2];
 node n;
 struct {node n;} hs[2];
 initial begin
  n=new(); hs[1].n=new();
  $display("this=%s direct=%0d module=%0d",n.name_value(),n.sarr[1].lvl,sarr[1].lvl);
  $display("handle=%0d name=%s",hs[1].n.arr[0],hs[1].n.arr[0].name());
 end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulation");

    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("this=L3 direct=3 module=0"), "{out}");
    assert!(out.contains("handle=1 name=L1"), "{out}");
}

#[test]
fn package_collection_kinds_keep_qualified_storage() {
    let src = r#"package p;
 typedef struct {int x; int y;} rec_t;
 int a[0:1]='{10,11};
 int q[$]='{2,3};
 int d[]=new[2];
 int aa[string]='{"k":4};
 rec_t r[2];
 int m[2][2];
 function int sum_a(); return a.sum(); endfunction
 class reader;
  function int read(); return a[1]; endfunction
 endclass
endpackage
module top;
 int a[2]='{100,200};
 int q[2]='{20,30};
 p::reader reader;
 initial begin
  reader=new(); p::r[1].x=7; p::m[1][1]=8; p::d[1]=6;
  p::q.push_back(5); p::aa["z"]=12;
  $display("values %0d %0d %0d %0d %0d %0d %0d %0d %0d",a[1],q[1],p::a[1],p::sum_a(),reader.read(),p::r[1].x,p::m[1][1],p::d[1],p::aa["k"]);
  $display("methods %0d %0d %0d",p::q.sum(),p::q.size(),p::aa.num());
 end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulation");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("values 200 30 11 21 11 7 8 6 4"), "{out}");
    assert!(out.contains("methods 10 3 2"), "{out}");
}

#[test]
fn struct_array_constructor_uses_field_type_without_repeating_index() {
    let src = r#"
module top;
 int calls=0;
 class child;
  int value;
  function new(int v); value=v; calls++; endfunction
 endclass
 struct {child item;} rows[2];
 int index=0;
 initial begin
  rows[index++].item=new(37);
  $display("constructed=%0d index=%0d calls=%0d",rows[0].item.value,index,calls);
 end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulation");
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("constructed=37 index=1 calls=1"), "{out}");
}
