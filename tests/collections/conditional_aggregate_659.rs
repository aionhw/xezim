//! IEEE 1800-2023 §11.4.11 / §11.2.2 / §7.6: conditional aggregate values.

#[test]
fn conditional_aggregates_preserve_values_in_assignment_contexts() {
    let sim = xezim::simulate(
        r#"
typedef struct { int x; int y; } pair_t;
class holder; int values[2]; endclass
module sink(input int values[2], output int total);
  always_comb total=values[0]+values[1];
endmodule
module top;
  int a[2]='{11,12}, b[2]='{21,22}, c[2]='{99,99};
  int da[]='{11,12}, db[]='{21,22}, q[$];
  pair_t sa='{1,2}, sb='{3,4}, sc;
  bit sel=1, clk;
  int calls, observed;
  holder h;
  sink u(sel?a:b,observed);
  function automatic bit choose(); calls++; return sel; endfunction
  always @(posedge clk) c<=sel?a:b;
  initial begin
    h=new;
    c=choose()?a:b;
    $display("T|fixed|%0d %0d %0d",c[0],c[1],calls);
    q=sel?da:db; sc=sel?sa:sb;
    h.values=sel?a:b;
    $display("T|other|%0d %0d %0d %0d",q.size(),q[1],sc.x,h.values[1]);
    begin
      int declared[2]=sel?a:b;
      $display("T|decl|%0d %0d",declared[0],declared[1]);
    end
    c=sel?(0?b:a):b;
    $display("T|nested|%0d %0d",c[0],c[1]);
    c=sel?'{31,32}:'{41,42};
    $display("T|pattern|%0d %0d",c[0],c[1]);
    $display("T|query|%0d %0d",$size(choose()?a:b),$bits(choose()?a:b));
    $display("T|query_calls|%0d",calls);
    #1 $display("T|port|%0d",observed);
    sel=0; #1 $display("T|port_false|%0d",observed);
    clk=1; #1 $display("T|nba|%0d %0d",c[0],c[1]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|fixed|11 12 1",
            "T|other|2 12 1 12",
            "T|decl|11 12",
            "T|nested|11 12",
            "T|pattern|31 32",
            "T|query|2 64",
            "T|query_calls|1",
            "T|port|23",
            "T|port_false|43",
            "T|nba|21 22",
        ]
    );
}

#[test]
fn ambiguous_aggregate_conditions_compare_whole_elements_once() {
    let sim = xezim::simulate(r#"
typedef struct { int x; int y; } pair_t;
typedef int ints_t[];
module top;
  logic [3:0] a[2]='{4'b1010,4'b1111}, b[2]='{4'b1010,4'b1100}, c[2];
  int ia[2]='{7,15}, ib[2]='{7,12}, ic[2];
  int da[]='{7,15}, db[]='{7,12}, dc[];
  pair_t sa='{7,15}, sb='{7,12}, sc;
  int calls, left_calls, right_calls;
  function automatic logic choose(); calls++; return 1'bx; endfunction
  function automatic ints_t left_values(); left_calls++; return da; endfunction
  function automatic ints_t right_values(); right_calls++; return db; endfunction
  initial begin
    c=choose()?a:b;
    $display("T|logic|%b %b %0d",c[0],c[1],calls);
    ic=choose()?ia:ib;
    $display("T|int|%0d %0d %0d",ic[0],ic[1],calls);
    sc=choose()?sa:sb;
    $display("T|struct|%0d %0d %0d",sc.x,sc.y,calls);
    dc=choose()?left_values():right_values();
    $display("T|dynamic|%0d %0d %0d %0d %0d %0d",dc.size(),dc[0],dc[1],calls,left_calls,right_calls);
    db=new[1]; dc=choose()?da:db;
    $display("T|length|%0d %0d",dc.size(),calls);
    ic=choose()?'{7,15}:'{7,12};
    $display("T|literal|%0d %0d %0d",ic[0],ic[1],calls);
    ic=2'b1x ? ia : ib;
    $display("T|true|%0d %0d",ic[0],ic[1]);
    ic<=choose()?ia:ib;
    #1 $display("T|nba|%0d %0d %0d",ic[0],ic[1],calls);
  end
endmodule
"#,100).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|logic|1010 xxxx 1",
            "T|int|7 0 2",
            "T|struct|0 0 3",
            "T|dynamic|2 7 0 4 1 1",
            "T|length|0 5",
            "T|literal|7 0 6",
            "T|true|7 15",
            "T|nba|7 0 7"
        ]
    );
}

#[test]
fn aggregate_arguments_and_returns_select_only_the_taken_arm() {
    let sim=xezim::simulate(r#"
typedef int ints_t[];
module top;
  int a[]='{11,12}, b[]='{21,22};
  int fa[2]='{3,4}, fb[2]='{5,6};
  int left_calls, right_calls;
  logic sel;
  function automatic ints_t left_values(); left_calls++; return a; endfunction
  function automatic ints_t right_values(); right_calls++; return b; endfunction
  function automatic ints_t values(); return sel?left_values():right_values(); endfunction
  function automatic int fixed_total(input int xs[2]); return xs[0]+xs[1]; endfunction
  function automatic int total(input int xs[]); return xs[0]+xs[1]; endfunction
  initial begin
    sel=1;
    $display("T|fixed_argument|%0d",fixed_total(sel?fa:fb));
    $display("T|argument|%0d %0d %0d",total(sel?left_values():right_values()),left_calls,right_calls);
    sel=0;
    begin int result[]=values(); $display("T|return|%0d %0d %0d %0d",result[0],result[1],left_calls,right_calls); end
    sel=1'bx;
    $display("T|unknown|%0d %0d %0d",total(sel?left_values():right_values()),left_calls,right_calls);
  end
endmodule
"#,100).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|fixed_argument|7",
            "T|argument|23 1 0",
            "T|return|21 22 1 1",
            "T|unknown|0 2 2"
        ]
    );
}

#[test]
fn conditional_aggregate_queries_keep_declared_dimensions() {
    let sim=xezim::simulate(r#"
module top;
  int a[3:2][0:1]='{'{1,2},'{3,4}};
  int b[3:2][0:1]='{'{1,2},'{7,8}};
  int c[3:2][0:1];
  localparam int CW=$bits(1 ? a : b);
  localparam int CN=$size(1 ? a : b);
  logic [$bits(1 ? a : b)-1:0] vector;
  int calls;
  function automatic bit choose(); calls++; return 0; endfunction
  initial begin
    $display("T|constant_query|%0d %0d %0d",CW,CN,$bits(vector));
    $display("T|dims|%0d %0d %0d %0d %0d",$size(choose()?a:b),$bits(choose()?a:b),$left(choose()?a:b),$right(choose()?a:b),$unpacked_dimensions(choose()?a:b));
    $display("T|calls|%0d",calls);
    c=1?a:b;
    $display("T|2d|%0d %0d",c[3][1],c[2][1]);
    c=1'bx ? a : b;
    $display("T|2dx|%0d %0d %0d %0d",c[3][0],c[3][1],c[2][0],c[2][1]);
  end
endmodule
"#,100).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|constant_query|128 2 128",
            "T|dims|2 128 3 2 2",
            "T|calls|0",
            "T|2d|2 4",
            "T|2dx|1 2 0 0"
        ]
    );
}

#[test]
fn ambiguous_struct_array_elements_default_as_whole_values() {
    let sim = xezim::simulate(
        r#"
typedef struct { int x; int y; } pair_t;
class holder; pair_t item; int values[2]; endclass
module top;
  pair_t a[2]='{'{1,2},'{3,4}}, b[2]='{'{1,2},'{3,8}}, c[2];
  holder left, right, result;
  initial begin
    left=new; right=new; result=new;
    c=a;
    $display("T|plain_struct_array|%0d %0d",c[0].x,c[0].y);
    c=1'bx ? a : b;
    $display("T|elements|%0d %0d %0d %0d",c[0].x,c[0].y,c[1].x,c[1].y);
    left.item='{1,2}; right.item='{1,8};
    result.item=1'bx ? left.item : right.item;
    $display("T|property|%0d %0d",result.item.x,result.item.y);
    left.values='{5,6}; right.values='{5,8};
    result.values=1'bx ? left.values : right.values;
    $display("T|property_array|%0d %0d",result.values[0],result.values[1]);
    c<=1'bx ? a : b;
    #1 $display("T|struct_array_nba|%0d %0d %0d %0d",c[0].x,c[0].y,c[1].x,c[1].y);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|plain_struct_array|1 2",
            "T|elements|1 2 0 0",
            "T|property|0 0",
            "T|property_array|5 0",
            "T|struct_array_nba|1 2 0 0"
        ]
    );
}

#[test]
fn conditional_aggregate_selectors_patterns_and_runtime_queries() {
    let sim = xezim::simulate(
        r#"
typedef struct {int x; int y;} pair_t;
module top;
  pair_t a[3]='{'{1,2},'{3,4},'{5,6}}, b[3]='{'{1,2},'{3,4},'{5,6}}, c;
  int i,j,k,calls,target_index;
  pair_t targets[2]='{'{99,99},'{99,99}};
  int fixed[2];
  int da[]='{1,2}, db[]='{3,4,5};
  function automatic bit choose(); calls++; return 0; endfunction
  initial begin
    c=1'bx ? a[i++] : b[j++];
    $display("T|indices|%0d %0d %0d %0d",i,j,c.x,c.y);
    c=1 ? a[k++] : b[j++];
    $display("T|known_index|%0d %0d %0d %0d",k,j,c.x,c.y);
    fixed=1'bx ? '{0:5,1:6} : '{0:5,1:8};
    $display("T|keyed|%0d %0d",fixed[0],fixed[1]);
    $display("T|size|%0d",$size(choose()?da:db));
    $display("T|calls|%0d",calls);
    targets[target_index++]=1'bx ? '{x:1,y:2} : '{x:1,y:8};
    $display("T|target|%0d %0d %0d %0d",target_index,targets[0].x,targets[0].y,targets[1].x);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|indices|1 1 1 2",
            "T|known_index|1 1 1 2",
            "T|keyed|5 0",
            "T|size|3",
            "T|calls|1",
            "T|target|1 0 0 99"
        ]
    );
}

#[test]
fn scalar_class_formal_hides_same_named_collection_metadata() {
    let sim = xezim::simulate(
        r#"
class item; int value; function new(int v); value=v; endfunction endclass
class chooser;
  static item original;
  static function item make(); item result=new(22); return result; endfunction
  static function void select(ref item q, input bit first);
    q=first?original:make();
  endfunction
endclass
module top;
  int q[$];
  item a;
  initial begin
    q.push_back(99);
    chooser::original=new(11);
    chooser::select(a,1); $display("T|class_then|%0d %0d",a.value,q.size());
    chooser::select(a,0); $display("T|class_else|%0d %0d",a.value,q.size());
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(lines, ["T|class_then|11 1", "T|class_else|22 1"]);
}

#[test]
fn contextual_patterns_bind_receiver_targets_and_array_formals() {
    let sim=xezim::simulate(r#"
class holder; int a[2]; int q[$]; int value; endclass
module top;
  holder h;
  int calls;
  logic sel;
  function automatic holder get(); calls++; return h; endfunction
  function automatic int fixed_total(input int a[2]); return a[0]+a[1]; endfunction
  function automatic int dynamic_total(input int a[]); return a[0]+a[1]; endfunction
  initial begin
    h=new; sel=1;
    get().a=sel ? '{1,2} : '{3,4};
    $display("T|receiver|%0d %0d %0d",calls,h.a[0],h.a[1]);
    sel=1'bx;
    get().a=sel ? '{1,2} : '{1,8};
    $display("T|receiver_unknown|%0d %0d %0d",calls,h.a[0],h.a[1]);
    get().q=sel ? '{1,2} : '{1,8};
    $display("T|receiver_queue|%0d %0d %0d %0d",calls,h.q.size(),h.q[0],h.q[1]);
    $display("T|formal_known|%0d %0d",fixed_total(1 ? '{1,2} : '{3,4}),dynamic_total(0 ? '{1,2} : '{3,4}));
    $display("T|formal_unknown|%0d %0d",fixed_total(sel ? '{1,2} : '{1,8}),dynamic_total(sel ? '{1,2} : '{1,8}));
    $display("T|formal_width|%0d",fixed_total(sel ? '{4'hf+4'h1,2} : '{16,2}));
    $display("T|formal_replication|%0d %0d",dynamic_total(1 ? '{2{3}} : '{4,5}),dynamic_total(sel ? '{2{3}} : '{3,5}));
    get().a='{5,6};
    $display("T|plain_receiver|%0d %0d %0d",calls,h.a[0],h.a[1]);
    get().a<=sel ? '{1,2} : '{1,8};
    #1 $display("T|receiver_nba|%0d %0d %0d",calls,h.a[0],h.a[1]);
    get().value=7;
    $display("T|scalar_receiver|%0d %0d",calls,h.value);
  end
endmodule
"#,100).expect("simulate");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|receiver|1 1 2",
            "T|receiver_unknown|2 1 0",
            "T|receiver_queue|3 2 1 0",
            "T|formal_known|3 7",
            "T|formal_unknown|1 1",
            "T|formal_width|18",
            "T|formal_replication|6 3",
            "T|plain_receiver|4 5 6",
            "T|receiver_nba|5 1 0",
            "T|scalar_receiver|6 7"
        ]
    );
}
