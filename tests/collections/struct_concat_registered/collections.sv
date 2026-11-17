// IEEE 1800-2023 §10.10: fixed, queue and dynamic targets; snapshot aliases.
module top;
  typedef struct {int id; string label;} item_t;
  typedef struct packed {int id;} packed_t;
  item_t b,c,mq[$],md[];
  packed_t pb,pc,pa[2];
  int scalar[4], middle[2];
  initial begin
    item_t q[$], da[];
    b.id=3; b.label="bee"; c.id=7; c.label="cee";
    q={b,c}; da={b,c}; mq={b,c}; md={b,c};
    $display("T|local|%0d %0d %s %0d %0d %s",q.size(),q[0].id,q[1].label,da.size(),da[0].id,da[1].label);
    q={q[1],q[0]}; da={da[1],da[0]};
    $display("T|reorder|%0d %s %0d %s",q[0].id,q[1].label,da[0].id,da[1].label);
    $display("T|module|%0d %0d %s %0d %0d %s",mq.size(),mq[0].id,mq[1].label,md.size(),md[0].id,md[1].label);
    mq={mq[1],mq[0]}; md={md[1],md[0]};
    $display("T|module_reorder|%0d %s %0d %s",mq[0].id,mq[1].label,md[0].id,md[1].label);
    pb.id=3; pc.id=7; pa={pb,pc};
    middle='{2,3}; scalar={1,middle,4};
    $display("T|guards|%0d %0d %0d %0d %0d %0d",pa[0].id,pa[1].id,scalar[0],scalar[1],scalar[2],scalar[3]);
    $finish;
  end
endmodule
