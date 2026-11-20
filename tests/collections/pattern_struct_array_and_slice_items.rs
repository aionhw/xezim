//! IEEE 1800-2023 §10.9.1, §10.10, §7.6, §7.4.6: an assignment pattern item
//! that is an array of unpacked structures supplies its elements member by
//! member, and an item that is a slice of an unpacked array supplies the
//! slice's elements, with or without an intra-assignment delay (§9.4.5). The
//! structure array stored zeros and the slice repeated its first element.

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn struct_array_and_slice_items_without_a_delay() {
    let out = lines(
        r#"
module top;
  typedef struct { int a; int b; } S;
  int wc [2] = '{1, 2};
  int big [4] = '{10, 11, 12, 13};
  int bd [3:0] = '{10, 11, 12, 13};
  int m [2][3] = '{'{1, 2, 3}, '{4, 5, 6}};
  S sa [2] = '{'{1, 2}, '{3, 4}};
  S s2 [2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{5, 6}, '{7, 8}}};
  int t3 [2][2], t4 [2][2], t5 [2][2];
  S ts [2][2], tr [2][2];
  S t7 [2][2][2];
  initial begin
    t3 = '{wc, big[1:2]};
    t4 = '{m[1][1:2], bd[2:1]};
    t5 = '{big[2+:2], wc};
    ts = '{sa, sa};
    tr = '{1: s2[1], 0: sa};
    t7 = '{s2, s2};
    $display("T|t3=%p t4=%p t5=%p", t3, t4, t5);
    $display("T|ts=%p", ts);
    $display("T|tr=%p", tr);
    $display("T|t7=%p", t7);
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|t3='{'{1, 2}, '{11, 12}} t4='{'{5, 6}, '{11, 12}} t5='{'{12, 13}, '{1, 2}}",
            "T|ts='{'{'{a:1, b:2}, '{a:3, b:4}}, '{'{a:1, b:2}, '{a:3, b:4}}}",
            "T|tr='{'{'{a:1, b:2}, '{a:3, b:4}}, '{'{a:5, b:6}, '{a:7, b:8}}}",
            "T|t7='{'{'{'{a:1, b:2}, '{a:3, b:4}}, '{'{a:5, b:6}, '{a:7, b:8}}}, \
             '{'{'{a:1, b:2}, '{a:3, b:4}}, '{'{a:5, b:6}, '{a:7, b:8}}}}",
        ]
    );
}

#[test]
fn struct_array_and_slice_items_with_a_delay() {
    let out = lines(
        r#"
module top;
  typedef struct { int a; int b; } S;
  int one [2] = '{1, 2};
  int src [2][2] = '{'{31, 32}, '{41, 42}};
  S sa [2] = '{'{1, 2}, '{3, 4}};
  S s2 [2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{5, 6}, '{7, 8}}};
  int p5 [2][2], p6 [2][2];
  S p4 [2][2], n4 [2][2];
  initial begin p4 = #4 '{sa, s2[1]}; $display("T|p4=%p", p4); end
  initial begin p5 = #4 '{src[1][0:1], one}; $display("T|p5=%p", p5); end
  initial begin p6 = #4 '{one[0:1], one}; $display("T|p6=%p", p6); end
  initial begin n4 <= #4 '{s2[0], sa}; #5 $display("T|n4=%p", n4); end
  initial begin
    #1 src = '{'{0, 0}, '{0, 0}}; one = '{0, 0};
    sa = '{'{0, 0}, '{0, 0}};
    s2 = '{'{'{0, 0}, '{0, 0}}, '{'{0, 0}, '{0, 0}}};
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|p4='{'{'{a:1, b:2}, '{a:3, b:4}}, '{'{a:5, b:6}, '{a:7, b:8}}}",
            "T|p5='{'{41, 42}, '{1, 2}}",
            "T|p6='{'{1, 2}, '{1, 2}}",
            "T|n4='{'{'{a:1, b:2}, '{a:3, b:4}}, '{'{a:1, b:2}, '{a:3, b:4}}}",
        ]
    );
}
