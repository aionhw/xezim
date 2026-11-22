//! §9.4.2: an event control in an automatic task that reads the task's local
//! `k` through a system function argument (`$signed(sa[k].a)`) is evaluated
//! with the waiting call's `k`, not with whichever call's frame is live. Two
//! calls wait at once with different `k`; each wakes once, when its own
//! member changes, and not on a change of another member of its element. The
//! expected values are the reference simulator's.

fn lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

#[test]
fn a_task_local_in_a_call_argument() {
    let src = r#"
module tb;
  typedef struct packed { logic [7:0] a; logic [7:0] b; } s_t;
  s_t sa[4];
  int n[4];
  task automatic w(int k);
    repeat (3) begin
      @($signed(sa[k].a));
      n[k]++;
    end
  endtask
  initial begin
    #1;
    fork w(1); w(2); join_none
  end
  initial begin
    for (int k = 0; k < 4; k++) sa[k] = '0;
    #2 sa[1].a = 8'h01;
    #1 sa[2].a = 8'h03;
    #1 sa[2].b = 8'h03;
    #1 sa[1].b = 8'h09;
    #1 $display("n=%0d,%0d", n[1], n[2]);
    $finish;
  end
endmodule
"#;
    assert_eq!(lines(src), ["n=1,1"]);
}
