`timescale 1ns/1ps
class Node;
  int id;
  Node next_node;
  function new(int n=0); id=n; endfunction
endclass
// The global name must not override a class's type parameter.
typedef int T;
class Box #(type T=Node);
  T item;
endclass
module top;
  mailbox #(Node) messages = new(1);
  Node array_roots[];
  int key_roots[Node];
  Box #(Node) typed_root;
  Node output_root;
  int checks;
  task automatic make_output(output Node result);
    result = new(71);
    #20;
  endtask
  task automatic wait_ref(ref Node item);
    #15;
    if (item == null || item.id != 33) $fatal(1,"ref root lost");
    checks++;
  endtask
  task automatic parked_local();
    Node only_local = new(44);
    #12;
    if (only_local.id != 44) $fatal(1,"parked local lost");
    checks++;
  endtask
  function automatic Node make_node(int id);
    Node fresh = new(id);
    return fresh;
  endfunction
  task automatic produce();
    // No caller local retains the second object while put is blocked.
    messages.put(make_node(55));
    messages.put(make_node(66));
  endtask
  task automatic shadowed_local();
    Node saved = new(99);
    begin
      Node saved = null;
      #14;
      if (saved != null) $fatal(1,"inner shadow changed");
    end
    if (saved == null || saved.id != 99) $fatal(1,"shadowed local lost");
    checks++;
  endtask
  task automatic consume();
    Node received;
    #10;
    messages.get(received);
    if (received.id != 55) $fatal(1,"mailbox root lost");
    #10;
    messages.get(received);
    if (received.id != 66) $fatal(1,"blocked put root lost");
    checks++;
  endtask
  initial begin
    Node temporary;
    Node cycle;
    Node ref_root = new(33);
    array_roots = new[1];
    temporary = new(11);
    key_roots[temporary] = 88;
    temporary = null;
    cycle = new(22);
    cycle.next_node = new(23);
    cycle.next_node.next_node = cycle;
    array_roots[0] = cycle;
    cycle = null;
    typed_root = new;
    typed_root.item = new(77);
    fork
      begin
        for (int n=0; n<30000; n++) begin
          temporary = new(1000+n);
          if ((n % 1000)==0) #1;
        end
        temporary = null;
      end
      make_output(output_root);
      wait_ref(ref_root);
      parked_local();
      shadowed_local();
      produce();
      consume();
    join
    foreach (key_roots[k]) begin
      if (k == null || k.id != 11 || key_roots[k] != 88) $fatal(1,"associative key lost");
      checks++;
    end
    if (array_roots[0].id != 22 || array_roots[0].next_node.id != 23 ||
        array_roots[0].next_node.next_node != array_roots[0]) $fatal(1,"cycle lost");
    if (typed_root.item.id != 77) $fatal(1,"type parameter root lost");
    if (output_root == null || output_root.id != 71) $fatal(1,"output root lost");
    if (checks != 5) $fatal(1,"wrong check count %0d", checks);
    $display("HEAP_LIFETIMES_PASS checks=%0d", checks);
    $finish;
  end
endmodule
