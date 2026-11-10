//! IEEE 1800-2023 §10.10: array concatenation preserves unpacked members.

fn check(src: &str) {
    let sim = xezim::simulate(src, 100).expect("simulate");
    assert!(
        sim.output.iter().any(|o| o.message == "T|PASS"),
        "{:?}",
        sim.output
    );
    assert!(
        !sim.output.iter().any(|o| o.message.contains("Fatal")),
        "{:?}",
        sim.output
    );
}

#[test]
fn struct_queue_concat_assignment_and_formals() {
    check(
        r#"
typedef struct {int addr; int len;} rec_t;
module top;
 function automatic int sum_q(rec_t f[$]);
   int s=0;
   foreach(f[i]) s+=f[i].addr+f[i].len;
   return f.size()*1000+s;
 endfunction
 initial begin
   rec_t q[$],r[$],e;
   e.addr=1; e.len=2; q.push_back(e);
   e.addr=3; e.len=4; q.push_back(e);
   if(sum_q(q)!=2010 || sum_q({q})!=2010) $fatal(1,"formal members");
   r={q};
   if(r.size()!=2 || r[1].addr!=3) $fatal(1,"assignment members");
   q={q[1],q[0]};
   if(q.size()!=2 || q[0].addr!=3 || q[1].addr!=1) $fatal(1,"alias reorder");
   q={q,e};
   if(q.size()!=3 || q[2].len!=4) $fatal(1,"self append");
   r={q[1:2]};
   if(r.size()!=2 || r[0].addr!=1 || r[1].addr!=3) $fatal(1,"slice");
   $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn struct_concat_preserves_nonintegral_and_nested_fields() {
    check(
        r#"
typedef struct {int id; string label;} inner_t;
typedef struct {inner_t item; real weight; int data[2];} rec_t;
class holder;
 rec_t values[$];
 function void set_values(rec_t v[$]); values={v}; endfunction
endclass
module top;
 function automatic bit valid(rec_t v[5:4]);
   return v[5].item.label=="first" && v[4].item.label=="second" &&
          v[5].weight==1.25 && v[4].data[1]==22;
 endfunction
 initial begin
   rec_t q[$],r[$],e;
   holder h=new;
   e.item.id=1; e.item.label="first"; e.weight=1.25;
   e.data[0]=11; e.data[1]=12; q.push_back(e);
   e.item.id=2; e.item.label="second"; e.weight=2.5;
   e.data[0]=21; e.data[1]=22; q.push_back(e);
   r={q}; h.set_values({q});
   if(!valid({q})) $fatal(1,"fixed formal or nested members");
   if(r[0].item.label!="first" || r[1].weight!=2.5 || r[0].data[1]!=12)
     $fatal(1,"nonintegral members");
   if(h.values.size()!=2 || h.values[1].item.label!="second")
     $fatal(1,"class collection");
   $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn struct_concat_copies_dynamic_and_associative_members_by_value() {
    check(
        r#"
typedef struct {int id; int data[]; string names[string];} rec_t;
module top;
 initial begin
   rec_t q[$], r[$], e;
   e.id=1; e.data=new[2]; e.data[0]=10; e.data[1]=11;
   e.names["key"]="value"; q.push_back(e);
   r={q};
   if(r.size()!=1 || r[0].data.size()!=2 || r[0].data[1]!=11 ||
      r[0].names.num()!=1 || r[0].names["key"]!="value")
     $fatal(1,"collection member copy");
   r[0].data[0]=99; r[0].names["key"]="changed";
   if(q[0].data[0]!=10 || q[0].names["key"]!="value")
     $fatal(1,"member alias");
   $display("T|PASS");
 end
endmodule
"#,
    );
}

#[test]
fn struct_return_operand_is_evaluated_once() {
    check(
        r#"
typedef struct {int a; int b;} rec_t;
module top;
 int calls;
 function automatic rec_t make(int n);
   rec_t value;
   calls++; value.a=n; value.b=n+1; return value;
 endfunction
 initial begin
   rec_t q[$];
   q={make(10),make(20)};
   $display("T|calls=%0d size=%0d a=%0d b=%0d",calls,q.size(),q[1].a,q[0].b);
   if(calls!=2 || q.size()!=2 || q[0].b!=11 || q[1].a!=20)
     $fatal(1,"struct call operand");
   $display("T|PASS");
 end
endmodule
"#,
    );
}
