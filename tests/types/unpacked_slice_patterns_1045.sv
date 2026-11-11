// IEEE 1800-2023 §7.4.5 / §10.9.1: an unpacked-array slice takes
// assignment-pattern items in left-to-right order at the element type.
class slice_parent;
  int dynamic_values[];
  int fixed_values[4];
  int descending_values[3:0];
  function new();
    dynamic_values = new[4];
    dynamic_values[0:3] = '{11, 12, 13, 14};
    fixed_values[0:3] = '{21, 22, 23, 24};
    descending_values[3:0] = '{31, 32, 33, 34};
  endfunction
  function void bare_write();
    dynamic_values[1:2] = '{41, 42};
    fixed_values[1:2] = '{43, 44};
  endfunction
  function void this_write();
    this.dynamic_values[0:1] = '{51, 52};
    this.fixed_values[0:1] = '{53, 54};
  endfunction
endclass
class slice_child extends slice_parent;
  function new();
    super.new();
    dynamic_values[0:1] = '{61, 62};
  endfunction
endclass
class slice_parameterized #(int N = 4);
  int values[N];
  function void fill();
    values[0+:2] = '{71, 72};
  endfunction
endclass
class slice_width #(int W = 9);
  logic [W-1:0] values[2];
  function void fill(input logic [7:0] a, b);
    values[0:1] = '{a + b, a + b};
  endfunction
endclass

module slice_peer;
  int values[4];
endmodule

module top;
  slice_peer peer();
  int ascending[0:3], descending[3:0], offset_values[5:8];
  int dynamic_values[];
  int overlap[0:3], matrix[2][3];
  int source[2];
  int nba_values[4];
  logic [8:0] wide_values[2];
  logic [7:0] a = 200, b = 100;
  string names[2];
  real reals[2];
  slice_parent obj;
  slice_child child;
  slice_parameterized #(6) sized;
  slice_width #(9) wide_obj;
  slice_parent saved, other;
  int failures = 0;
  int base = 1;
  task automatic check(input string name, input logic ok);
    if (ok !== 1'b1) begin
      $display("FAIL %s", name);
      failures++;
    end
  endtask
  task automatic local_arrays();
    int values[];
    values = new[4];
    values[0:1] = '{131, 132};
    peer.values[0:1] = '{141, 142};
    check("local dynamic", values[0] === 131 && values[1] === 132);
    check("hierarchy shadows local", peer.values[0] === 141 && peer.values[1] === 142);
  endtask
  initial begin
    obj = new;
    child = new;
    sized = new;
    wide_obj = new;
    local_arrays();
    check("constructor dynamic", obj.dynamic_values[0] === 11 && obj.dynamic_values[3] === 14);
    check("constructor fixed", obj.fixed_values[0] === 21 && obj.fixed_values[3] === 24);
    check("constructor descending", obj.descending_values[3] === 31 && obj.descending_values[0] === 34);
    check("inherited constructor", child.dynamic_values[0] === 61 && child.dynamic_values[1] === 62);
    ascending[0:3] = '{1, 2, 3, 4};
    descending[3:0] = '{5, 6, 7, 8};
    offset_values[5:8] = '{9, 10, 11, 12};
    check("ascending", ascending[0] === 1 && ascending[3] === 4);
    check("descending", descending[3] === 5 && descending[0] === 8);
    check("nonzero bounds", offset_values[5] === 9 && offset_values[8] === 12);
    dynamic_values = new[4];
    dynamic_values[0:3] = '{1, 2, 3, 4};
    check("dynamic", dynamic_values[0] === 1 && dynamic_values[3] === 4);
    ascending[base+:2] = '{13, 14};
    descending[2-:2] = '{15, 16};
    check("indexed ascending", ascending[0] === 1 && ascending[1] === 13 && ascending[2] === 14 && ascending[3] === 4);
    check("indexed descending", descending[2] === 15 && descending[1] === 16);
    dynamic_values[1:2] = '{default:17};
    check("default pattern", dynamic_values[0] === 1 && dynamic_values[1] === 17 && dynamic_values[2] === 17 && dynamic_values[3] === 4);
    ascending[1:2] = '{2:19, 1:18};
    check("keyed pattern", ascending[1] === 18 && ascending[2] === 19);
    descending[2:1] = '{2{20}};
    check("replicated pattern", descending[2] === 20 && descending[1] === 20);
    obj.dynamic_values[0:1] = '{81, 82};
    obj.fixed_values[0:1] = '{83, 84};
    check("handle dynamic", obj.dynamic_values[0] === 81 && obj.dynamic_values[1] === 82);
    check("handle fixed", obj.fixed_values[0] === 83 && obj.fixed_values[1] === 84);
    obj.bare_write();
    check("bare dynamic", obj.dynamic_values[1] === 41 && obj.dynamic_values[2] === 42);
    check("bare fixed", obj.fixed_values[1] === 43 && obj.fixed_values[2] === 44);
    obj.this_write();
    check("this dynamic", obj.dynamic_values[0] === 51 && obj.dynamic_values[1] === 52);
    check("this fixed", obj.fixed_values[0] === 53 && obj.fixed_values[1] === 54);
    sized.fill();
    check("parameterized", sized.values[0] === 71 && sized.values[1] === 72);
    wide_obj.fill(a, b);
    check("specialized element width", wide_obj.values[0] === 9'd300 && wide_obj.values[1] === 9'd300);
    wide_obj.values[0:1] = '{a + b, a + b};
    check("specialized handle width", wide_obj.values[0] === 9'd300 && wide_obj.values[1] === 9'd300);
    overlap[0:3] = '{1, 2, 3, 4};
    overlap[0:1] = '{overlap[1], overlap[0]};
    check("snapshot source", overlap[0] === 2 && overlap[1] === 1);
    matrix[1][0:2] = '{91, 92, 93};
    check("row slice", matrix[1][0] === 91 && matrix[1][2] === 93);
    wide_values[0:1] = '{a + b, a + b};
    check("element context", wide_values[0] === 9'd300 && wide_values[1] === 9'd300);
    names[0:1] = '{"a", "xyz"};
    check("string elements", names[0] == "a" && names[1] == "xyz");
    reals[0:1] = '{1.5, 2.75};
    check("real elements", reals[0] == 1.5 && reals[1] == 2.75);
    source = '{101, 102};
    dynamic_values[1:2] = source;
    check("dynamic copy control", dynamic_values[1] === 101 && dynamic_values[2] === 102);
    for (int i = 0; i < 1200; i++) obj.bare_write();
    check("hot method", obj.dynamic_values[1] === 41 && obj.fixed_values[2] === 44);
    nba_values[0:3] <= '{111, 112, 113, 114};
    saved = obj;
    other = new;
    obj.fixed_values[0:1] <= '{121, 122};
    obj = other;
    #1;
    check("nonblocking", nba_values[0] === 111 && nba_values[3] === 114);
    check("nonblocking receiver", saved.fixed_values[0] === 121 && saved.fixed_values[1] === 122 && other.fixed_values[0] === 21);
    if (failures) $fatal(1, "FAIL count=%0d", failures);
    $display("PASS unpacked slice patterns");
    $finish;
  end
endmodule
