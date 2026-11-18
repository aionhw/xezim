//! Compiled methods: whole-collection reads and writes of string queue,
//! dynamic, fixed and associative array members (§7.4, §7.5, §7.8, §7.10),
//! alone and in an unpacked array concatenation (§10.10).

use xezim::simulate;

fn run_lines(body: &str) -> Vec<String> {
    let _ = super::compiled_method_test_env::eager();
    let src = format!(
        r#"
module top;
  class holder;
    string list[$];
  endclass
  class C;
    string stream[$];
    string own[$];
    string dyn[];
    string fix[2];
    string fix2[2];
    string aa[int];
    string aa2[int];
    holder progs[1];
    function void run();
      {body}
    endfunction
  endclass
  initial begin
    automatic C c = new;
    c.progs[0] = new;
    c.stream.push_back("start");
    c.own.push_back("a");
    c.own.push_back("b");
    c.progs[0].list.push_back("x");
    c.progs[0].list.push_back("y");
    c.dyn = new[1];
    c.dyn[0] = "d0";
    c.fix[1] = "f1";
    c.aa[3] = "a3";
    c.run();
    $display("T|stream=%0d last=%s list=%0d dyn=%0d fix2=%s aa2=%0d", c.stream.size(),
             c.stream[$], c.progs[0].list.size(), c.dyn.size(), c.fix2[1], c.aa2.num());
  end
endmodule
"#
    );
    simulate(&src, 50)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn compiled_method_string_queue_member_concat() {
    assert_eq!(
        run_lines("stream = {stream, own};"),
        vec!["T|stream=3 last=b list=2 dyn=1 fix2= aa2=0"]
    );
}

#[test]
fn compiled_method_string_queue_member_concat_through_handle() {
    assert_eq!(
        run_lines("stream = {stream, progs[0].list};"),
        vec!["T|stream=3 last=y list=2 dyn=1 fix2= aa2=0"]
    );
}

#[test]
fn compiled_method_string_queue_member_store_through_handle() {
    assert_eq!(
        run_lines(r#"progs[0].list = {progs[0].list, "z"};"#),
        vec!["T|stream=1 last=start list=3 dyn=1 fix2= aa2=0"]
    );
}

#[test]
fn compiled_method_string_dyn_array_member_concat() {
    assert_eq!(
        run_lines(r#"dyn = {dyn, "d1"};"#),
        vec!["T|stream=1 last=start list=2 dyn=2 fix2= aa2=0"]
    );
}

#[test]
fn compiled_method_string_member_whole_copies() {
    assert_eq!(
        run_lines("stream = own; fix2 = fix; aa2 = aa;"),
        vec!["T|stream=2 last=b list=2 dyn=1 fix2=f1 aa2=1"]
    );
}
