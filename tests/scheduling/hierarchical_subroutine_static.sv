module leaf #(parameter BASE=7)(input logic clk);
  int observed;
  task automatic put(input int value);
    static bit [31:0] k;
    k=value;
    observed=k;
  endtask
  task bump;
    bit [31:0] k;
    k++;
  endtask
  task automatic recurse(input int depth);
    static bit [31:0] k;
    automatic int saved=depth;
    k++;
    if(depth) recurse(depth-1);
    if(saved!=depth) $fatal(1,"automatic frame changed");
  endtask
  always @(posedge clk) put(BASE);
  initial begin
    #2;
    put.k=BASE+100;
    #1;
    if(put.k!==BASE+100) $fatal(1,"relative instance scope");
  end
endmodule

module top;
  logic clk=0;
  always #5 clk=~clk;
  leaf #(7) a(clk);
  leaf #(9) b(clk);
  int automatic_result;
  int random_value;
  task automatic bounds;
    static bit [31:0] k;
    k=23;
  endtask
  task automatic matched;
    static bit [31:0] k;
    k=37;
  endtask
  task put(input int value);
    bit [31:0] k;
    k=value;
  endtask
  function int store(input int value);
    bit [31:0] k;
    k=value;
    return k;
  endfunction
  task automatic automatic_local(input int value);
    bit [31:0] k;
    k=value;
    automatic_result=k;
  endtask
  task static named;
    begin : left
      bit [31:0] k;
      k=13;
    end
    begin : right
      bit [31:0] k;
      k=17;
    end
  endtask
  initial begin
    if(a.put.k!==0 || b.bump.k!==0 || a.recurse.k!==0 || store.k!==0)
      $fatal(1,"static storage missing before first call");
    a.bump.k=10;
    put(101);
    bounds();
    matched();
    if(!(std::randomize(random_value) with { random_value == bounds.k; }))
      $fatal(1,"constraint-only static reference");
    if(random_value!==23) $fatal(1,"constraint static value");
    if(matched.k matches 37) begin end
    else $fatal(1,"matches-only static reference");
    a.bump();
    b.bump();
    a.recurse(3);
    if(store(21)!==21) $fatal(1,"function result");
    automatic_local(31);
    named();
    @(posedge clk);
    #1;
    if(a.observed!==7 || b.observed!==9 || a.put.k!==7 || $root.top.b.put.k!==9)
      $fatal(1,"clocked static storage");
    a.put.k[0]=0;
    if(a.put.k!==6 || b.put.k!==9 || a.bump.k!==11 || b.bump.k!==1
       || a.recurse.k!==4 || b.recurse.k!==0 || store.k!==21
       || named.left.k!==13 || named.right.k!==17 || automatic_result!==31 || put.k!==101)
      $fatal(1,"static hierarchy mismatch");
    $display("PASS hierarchical subroutine statics");
    $finish;
  end
endmodule
