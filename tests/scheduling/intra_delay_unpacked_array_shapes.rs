//! IEEE 1800-2023 §9.4.5, §7.6, §7.4.6: an intra-assignment delay or event
//! control reads an unpacked array right-hand side when the statement runs,
//! whatever its shape: a subarray of a descending or three-dimensional
//! array, a whole multidimensional array, a slice, an array of unpacked
//! structures or a row of one, a class property or a task local, into a
//! fixed, queue or dynamic-array target, blocking or nonblocking. Each was
//! read as one scalar and the store wrote zeros (#1320). The same copies
//! without a timing control are checked too: a subarray selected by two
//! indices, a row into a queue or dynamic array, and a nonblocking row copy
//! into a one-dimensional array stored zeros or nothing.

fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|entry| entry.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

#[test]
fn delayed_subarrays_slices_and_whole_arrays() {
    let out = lines(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int dsc[1:0][3:2] = '{'{1, 2}, '{3, 4}};
  int s3[2][2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{51, 52}, '{53, 54}}};
  int one[3] = '{7, 8, 9};
  logic [7:0] lp[2][2] = '{'{8'h11, 8'h22}, '{8'h33, 8'h44}};
  int a1[2], sl[2], sm[2];
  int t2[2][2], t3[2][2];
  logic [7:0] lq[2];
  initial begin a1 = #4 dsc[0]; $display("T|a1=%p", a1); end
  initial begin t2 = #4 s3[1]; $display("T|t2=%p", t2); end
  initial begin t3 = #4 dsc; $display("T|t3=%p", t3); end
  initial begin sl = #4 src[1][0:1]; $display("T|sl=%p", sl); end
  initial begin sm = #4 one[1:2]; $display("T|sm=%p", sm); end
  initial begin lq = #4 lp[1]; $display("T|lq=%h %h", lq[0], lq[1]); end
  initial begin
    #1 src = '{'{0, 0}, '{0, 0}}; dsc = '{'{0, 0}, '{0, 0}}; s3 = '{default: 0};
    one = '{0, 0, 0}; lp[1] = '{0, 0};
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|a1='{3, 4}",
            "T|t2='{'{51, 52}, '{53, 54}}",
            "T|t3='{'{1, 2}, '{3, 4}}",
            "T|sl='{41, 42}",
            "T|sm='{8, 9}",
            "T|lq=33 44",
        ]
    );
}

#[test]
fn delayed_rows_into_a_queue_and_a_dynamic_array() {
    let out = lines(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int qd[$], dy[];
  initial begin qd = #4 src[1]; $display("T|qd=%p", qd); end
  initial begin dy = #4 src[0]; $display("T|dy=%p", dy); end
  initial #1 src = '{'{0, 0}, '{0, 0}};
endmodule
"#,
    );
    assert_eq!(out, ["T|qd='{41, 42}", "T|dy='{31, 32}"]);
}

#[test]
fn delayed_arrays_of_unpacked_structures() {
    let out = lines(
        r#"
module top;
  typedef struct { int a; int b; } S;
  S sa[2] = '{'{1, 2}, '{3, 4}};
  S s2[2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{5, 6}, '{7, 8}}};
  S tb[2], tc[2], nbs[2];
  initial begin tb = #4 sa; $display("T|tb=%p", tb); end
  initial begin tc = #4 s2[1]; $display("T|tc=%p", tc); end
  initial begin nbs <= #4 s2[0]; #5 $display("T|nbs=%p", nbs); end
  initial begin
    #1 sa = '{'{0, 0}, '{0, 0}};
    s2 = '{'{'{0, 0}, '{0, 0}}, '{'{0, 0}, '{0, 0}}};
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|tb='{'{a:1, b:2}, '{a:3, b:4}}",
            "T|tc='{'{a:5, b:6}, '{a:7, b:8}}",
            "T|nbs='{'{a:1, b:2}, '{a:3, b:4}}",
        ]
    );
}

#[test]
fn delayed_nonblocking_and_event_controlled_subarrays() {
    let out = lines(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int s3[2][2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{51, 52}, '{53, 54}}};
  int nb1[2], nb2[2][2], nb3[2], ev[2];
  event go;
  initial begin
    nb1 <= #4 s3[1][0];
    nb2 <= #4 s3[0];
    nb3 <= @(go) src[0];
    #5 $display("T|nb1=%p nb2=%p nb3=%p", nb1, nb2, nb3);
  end
  initial begin ev = @(go) s3[1][1]; $display("T|ev=%p", ev); end
  initial begin
    #1 src = '{'{0, 0}, '{0, 0}}; s3 = '{default: 0};
    #1 -> go;
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T|ev='{53, 54}",
            "T|nb1='{51, 52} nb2='{'{1, 2}, '{3, 4}} nb3='{31, 32}",
        ]
    );
}

#[test]
fn delayed_class_property_and_task_local_rows() {
    let out = lines(
        r#"
module top;
  class K;
    int km[2][2];
    int kr[2];
    int kt[2][2];
    function new(); km = '{'{1, 2}, '{3, 4}}; endfunction
    task row(); kr = #2 km[1]; endtask
    task whole(); kt = #2 km; endtask
  endclass
  K k;
  task automatic tl(output int o[2]);
    int l[2][2];
    l = '{'{5, 6}, '{7, 8}};
    fork #1 l[1] = '{0, 0}; join_none
    o = #3 l[1];
  endtask
  initial begin
    k = new;
    fork k.row(); k.whole(); join
    $display("T|kr=%p kt=%p", k.kr, k.kt);
  end
  initial begin int o[2]; tl(o); $display("T|tl=%p", o); end
  initial #1 k.km = '{default: 0};
endmodule
"#,
    );
    assert_eq!(out, ["T|kr='{3, 4} kt='{'{1, 2}, '{3, 4}}", "T|tl='{7, 8}"]);
}

#[test]
fn undelayed_subarray_copies_into_arrays_queues_and_dynamic_arrays() {
    let out = lines(
        r#"
module top;
  int src[2][2] = '{'{31, 32}, '{41, 42}};
  int s3[2][2][2] = '{'{'{1, 2}, '{3, 4}}, '{'{51, 52}, '{53, 54}}};
  int q[2], e[2], qq[$], dd[];
  initial begin
    q <= src[1];
    e = s3[1][0];
    qq = src[1];
    dd = src[0];
    #1 $display("T|q=%p e=%p qq=%p dd=%p", q, e, qq, dd);
  end
endmodule
"#,
    );
    assert_eq!(out, ["T|q='{41, 42} e='{51, 52} qq='{41, 42} dd='{31, 32}"]);
}

#[test]
fn delayed_row_selector_is_evaluated_once_at_capture() {
    let source = r#"
module top;
  int rows[2][2] = '{'{2, 3}, '{5, 7}};
  int a[2], b[2], calls;
  function automatic int choose();
    calls++;
    return 1;
  endfunction
  initial begin
    b <= #4 rows[choose()];
    a = #4 rows[choose()];
    #1 $display("T|%0d %0d | %0d %0d | %0d", a[0], a[1], b[0], b[1], calls);
  end
  initial #1 rows[1] = '{11, 13};
endmodule
"#;
    assert_eq!(lines(source), ["T|5 7 | 5 7 | 2"]);
}
