//! §8.3 / §7.10 / §10.8: inherited collection stores and formal member context.

fn tags(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate regression");
    assert_eq!(sim.error_count, 0);
    sim.output
        .iter()
        .filter(|line| line.message.starts_with("T|"))
        .map(|line| line.message.clone())
        .collect()
}

#[test]
fn inherited_nonempty_patterns_use_the_declaring_collection_metadata() {
    assert_eq!(
        tags(
            r#"
class base;
  int iq[$]; string sq[$]; byte da[]; int aa[int];
endclass
class child extends base; endclass
class shadow extends base; bit[7:0] iq[$]; endclass
module top;
  initial begin
    child c = new(); shadow s = new();
    c.iq = '{6,7}; c.sq = '{"a","b"}; c.da = '{1,2,3};
    c.aa = '{1:12,default:55};
    s.iq = '{16'h1ff};
    $display("T|inherited|%0d|%0d|%0d|%s|%0d|%0d|%0d|%0d",c.iq.size(),c.iq[0],c.sq.size(),c.sq[1],c.da.size(),c.da[2],c.aa[1],c.aa[9]);
    $display("T|shadow|%0d|%0d",s.iq.size(),s.iq[0]);
    c.iq = '{}; c.sq = '{}; c.da = '{};
    $display("T|empty|%0d|%0d|%0d",c.iq.size(),c.sq.size(),c.da.size());
    $finish;
  end
endmodule
"#
        ),
        [
            "T|inherited|2|6|2|b|3|3|12|55",
            "T|shadow|1|255",
            "T|empty|0|0|0"
        ]
    );
}

#[test]
fn positional_struct_patterns_keep_formal_width_in_each_call_path() {
    assert_eq!(
        tags(
            r#"
typedef struct {bit[15:0] a; bit[31:0] b;} pair_t;
class regular;
 function void show(pair_t s);$display("T|method|%h|%h",s.a,s.b);endfunction
endclass
class generic #(type T=pair_t);
 function void show(T s);$display("T|generic|%h|%h",s.a,s.b);endfunction
endclass
module top;
 function automatic void show(pair_t s);$display("T|function|%h|%h",s.a,s.b);endfunction
 initial begin
  regular c=new();generic #(pair_t) g=new();
  show('{8'hff+8'h01,1'b1+1'b1});
  show('{a:8'hff+8'h01,b:1'b1+1'b1});
  c.show('{8'hff+8'h01,1'b1+1'b1});
  g.show('{8'hff+8'h01,1'b1+1'b1});
  $finish;
 end
endmodule
"#
        ),
        [
            "T|function|0100|00000002",
            "T|function|0100|00000002",
            "T|method|0100|00000002",
            "T|generic|0100|00000002"
        ]
    );
}
