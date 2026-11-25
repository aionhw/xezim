//! IEEE 1800-2023 §7.2.1, §7.6, and §9.4.5. A delayed assignment
//! samples the whole unpacked value, including its collection members.

use xezim::simulate;

fn check_queue_copy(source: &str, destination: &str, assignment: &str, setup: &str) {
    let src = format!(
        r#"
module top;
  typedef struct {{ int q[$]; }} Inner;
  typedef struct {{ int tag; Inner payload; }} S;
  class C;
    S src[2], dst[2], one, two, seq[$], result[$];
  endclass
  C c;
  S a[2], b[2], one, two, a_seq[$], b_seq[$];
  event gate;
  initial begin
    c = new;
    {setup}
    {source}.tag = 7;
    {source}.payload.q.push_back(8);
    {source}.payload.q.push_back(9);
    {destination}.payload.q.push_back(55);
    fork
      begin {assignment} end
      begin
        #1 {source}.payload.q[0] = 20;
        {source}.payload.q.push_back(10);
        #2 ->gate;
      end
    join
    $display("sample %0d %0d %0d", {destination}.tag,
        {destination}.payload.q.size(), {destination}.payload.q[0]);
  end
endmodule
"#
    );
    let result = simulate(&src, 100).expect("simulate failed");
    assert_eq!(
        result
            .output
            .iter()
            .map(|entry| entry.message.as_str())
            .collect::<Vec<_>>(),
        ["sample 7 2 8"]
    );
}

#[test]
fn module_struct_captures_a_nested_queue() {
    check_queue_copy("one", "two", "two = #3 one;", "");
}

#[test]
fn class_struct_captures_a_nested_queue() {
    check_queue_copy("c.one", "c.two", "c.two = #3 c.one;", "");
}

#[test]
fn fixed_module_array_captures_a_nested_queue() {
    check_queue_copy("a[0]", "b[0]", "b = #3 a;", "");
}

#[test]
fn fixed_class_array_captures_a_nested_queue() {
    check_queue_copy("c.src[0]", "c.dst[0]", "c.dst = #3 c.src;", "");
}

#[test]
fn event_control_captures_a_nested_queue() {
    check_queue_copy("a[0]", "b[0]", "b = @(gate) a;", "");
}

#[test]
fn sliced_destination_captures_a_nested_queue() {
    check_queue_copy("c.src[1]", "b[0]", "b[0:0] = #3 c.src[1:1];", "");
}

#[test]
fn class_element_captures_a_nested_queue() {
    check_queue_copy(
        "c.seq[0]",
        "c.two",
        "c.two = #3 c.seq[0];",
        "c.seq.push_back('{default:0});",
    );
}

#[test]
fn queue_of_structs_captures_nested_queues() {
    check_queue_copy(
        "c.seq[0]",
        "c.result[0]",
        "c.result = #3 c.seq;",
        "c.seq.push_back('{default:0}); c.result.push_back('{default:0});",
    );
}

#[test]
fn array_copy_preserves_a_nested_dynamic_array() {
    let src = r#"
module top;
  typedef struct { int tag; int values[]; } S;
  class C; S src[2], dst[2]; endclass
  C c;
  initial begin
    c = new;
    c.src[0].tag = 7;
    c.src[0].values = new[2];
    c.src[0].values[0] = 8;
    c.src[0].values[1] = 9;
    c.dst[0].values = new[1];
    c.dst[0].values[0] = 55;
    fork
      c.dst = #3 c.src;
      #1 c.src[0].values[0] = 20;
    join
    $display("sample %0d %0d %0d", c.dst[0].tag, c.dst[0].values.size(),
        c.dst[0].values[0]);
  end
endmodule
"#;
    let result = simulate(src, 100).expect("simulate failed");
    assert_eq!(result.output[0].message, "sample 7 2 8");
}

#[test]
fn array_copy_preserves_associative_keys_and_replaces_old_entries() {
    let src = r#"
module top;
  typedef struct { int numbers[int]; int words[string]; } S;
  S src[2], dst[2];
  initial begin
    src[0].numbers[-2] = 8;
    src[0].numbers[5] = 9;
    src[0].words["a]b"] = 11;
    dst[0].numbers[1] = 55;
    dst[0].words["old"] = 77;
    fork
      dst = #3 src;
      #1 begin
        src[0].numbers[-2] = 20;
        src[0].words["a]b"] = 22;
      end
    join
    $display("sample %0d %0d %0d %0d", dst[0].numbers.num(),
        dst[0].numbers[-2], dst[0].words.num(), dst[0].words["a]b"]);
  end
endmodule
"#;
    let result = simulate(src, 100).expect("simulate failed");
    assert_eq!(result.output[0].message, "sample 2 8 1 11");
}
