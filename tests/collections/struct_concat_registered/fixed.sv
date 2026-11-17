// IEEE 1800-2023 §10.10, §7.2: each struct operand contributes one element.
module top;
  typedef struct packed {logic [3:0] tag; bit [3:0] code;} packed_t;
  typedef struct {int id; string label;} inner_t;
  typedef struct {inner_t item; real weight; packed_t bits; int data[2];} item_t;
  item_t a[3], b, c, d, control;
  item_t reverse[5:3];
  initial begin
    b.item.id=3; b.item.label="bee"; b.weight=1.25; b.bits=8'h35;
    b.data[0]=11; b.data[1]=12;
    c.item.id=7; c.item.label="cee"; c.weight=2.5; c.bits=8'h79;
    c.data[0]=21; c.data[1]=22;
    d.item.id=-9; d.item.label="a much longer third string";
    d.weight=-3.75; d.bits=8'hdb; d.data[0]=31; d.data[1]=32;
    a[0].item.id=91; a[0].item.label="old0";
    a[1].item.id=92; a[1].item.label="old1";
    a[2].item.id=93; a[2].item.label="old2";
    control=b;
    $display("T|control|%0d %s",control.item.id,control.item.label);
    a={b,c,d};
    $display("T|ids|%0d %0d %0d",a[0].item.id,a[1].item.id,a[2].item.id);
    $display("T|labels|%s %s %s",a[0].item.label,a[1].item.label,a[2].item.label);
    $display("T|reals|%0.2f %0.2f %0.2f",a[0].weight,a[1].weight,a[2].weight);
    $display("T|packed|%h %h %h",a[0].bits,a[1].bits,a[2].bits);
    $display("T|packed_members|%0d %0d %0d %0d %0d %0d",a[0].bits.tag,a[0].bits.code,a[1].bits.tag,a[1].bits.code,a[2].bits.tag,a[2].bits.code);
    $display("T|data|%0d %0d %0d",a[0].data[1],a[1].data[1],a[2].data[1]);
    a={a[2],a[0],a[1]};
    $display("T|alias|%0d %0d %0d",a[0].item.id,a[1].item.id,a[2].item.id);
    reverse={b,c,d};
    $display("T|descending|%0d %0d %0d",reverse[5].item.id,reverse[4].item.id,reverse[3].item.id);
    $finish;
  end
endmodule
