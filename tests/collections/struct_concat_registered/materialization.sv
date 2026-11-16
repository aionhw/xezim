// IEEE 1800-2023 §10.9.1, §10.9.2, §11.4.11: patterns and conditional arrays.
module top;
  typedef struct {bit signed [31:0] id; string label;} item_t;
  typedef item_t pair_t[2];
  pair_t a,x,y;
  item_t b,c;
  bit cond;
  function automatic int total(input item_t values[2]);
    return values[0].id*10+values[1].id;
  endfunction
  initial begin
    b.id=3; b.label="bee"; c.id=7; c.label="cee";
    x[0]=b; x[1]=c; y[0]=c; y[1]=b;
    a='{'{3,"bee"},'{7,"cee"}};
    $display("T|pattern|%0d %s %0d %s",a[0].id,a[0].label,a[1].id,a[1].label);
    a=pair_t'{'{7,"cee"},'{3,"bee"}};
    $display("T|typed|%0d %s %0d %s",a[0].id,a[0].label,a[1].id,a[1].label);
    cond=1; a=cond?x:y;
    $display("T|true|%0d %s %0d %s",a[0].id,a[0].label,a[1].id,a[1].label);
    cond=0; a=cond?x:y;
    $display("T|false|%0d %s %0d %s",a[0].id,a[0].label,a[1].id,a[1].label);
    a=cond?pair_t'{'{3,"bee"},'{7,"cee"}}:pair_t'{'{7,"cee"},'{3,"bee"}};
    $display("T|conditional_pattern|%0d %s %0d %s",a[0].id,a[0].label,a[1].id,a[1].label);
    $display("T|typed_formal|%0d",total(pair_t'{'{3,"bee"},'{7,"cee"}}));
    a=1'bx ? x : y;
    $display("T|ambiguous|%0d %0d",a[0].id,a[1].id);
    $finish;
  end
endmodule
