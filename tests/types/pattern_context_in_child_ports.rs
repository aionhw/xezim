//! §10.8, §10.9.1, §10.9.2, §23.3.3: a pattern assigned to a child module's
//! OUTPUT PORT takes the declared type of the net the port is collapsed onto,
//! and a `default:` whose value is a parameter of the child (`c.LOC`) or a
//! package parameter imported bare (`RST_REQ` for `p::RST_REQ`) is assigned
//! whole when its declared type matches the element (§6.22.1). The port's
//! type was rebuilt from its flattened member layout, which lost the
//! dimension of a `req_t [1:0]` member, and the parameters' declared types
//! were looked up under the wrong keys, so the value was split by member.
//! Expected values agree with the reference simulator.
use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

const PKG: &str = "package p;
  typedef enum logic [1:0] {IDLE=0, RUN=1, DONE=2} st_t;
  typedef struct packed { logic [7:0] addr; st_t st; logic v; } req_t;
  typedef struct packed { req_t [1:0] rq; logic [3:0] tag; } bundle_t;
  typedef struct packed { logic [1:0][10:0] rq; logic [3:0] tag; } flat_t;
  localparam req_t RST_REQ = '{addr: 8'hFF, st: DONE, v: 1'b1};
endpackage
";

#[test]
fn child_port_pattern_takes_the_declared_struct_type() {
    let out = lines(&format!(
        "{PKG}
module child (input logic [3:0] a, output p::bundle_t pb, output p::flat_t pf, output logic [25:0] lb_o, lc_o);
  import p::*;
  bundle_t lb;
  typedef struct packed {{ logic [7:0] addr; logic [1:0] st; logic v; }} lreq_t;
  typedef struct packed {{ lreq_t [1:0] rq; logic [3:0] tag; }} lbundle_t;
  lbundle_t lc;
  always_comb pb = '{{rq: '{{11'h7fd, 11'h772}}, tag: a}};
  always_comb pf = '{{rq: '{{11'h7fd, 11'h772}}, tag: a}};
  always_comb lb = '{{rq: '{{11'h7fd, 11'h772}}, tag: a}};
  always_comb lc = '{{rq: '{{11'h7fd, 11'h772}}, tag: a}};
  assign lb_o = lb; assign lc_o = lc;
endmodule
module top;
  import p::*;
  logic [3:0] a = 4'h3;
  bundle_t pb, tb; flat_t pf; logic [25:0] lb, lc;
  child c(.a(a), .pb(pb), .pf(pf), .lb_o(lb), .lc_o(lc));
  always_comb tb = '{{rq: '{{11'h7fd, 11'h772}}, tag: a}};
  initial begin
    #1 $display(\"T|pb=%h pf=%h lb=%h lc=%h tb=%h\", pb, pf, lb, lc, tb);
  end
endmodule
"
    ));
    assert_eq!(
        out,
        ["T|pb=3fef723 pf=3fef723 lb=3fef723 lc=3fef723 tb=3fef723"]
    );
}

#[test]
fn default_of_a_scoped_or_imported_parameter_assigns_whole() {
    let out = lines(&format!(
        "{PKG}
module child (input logic [3:0] a, output p::bundle_t b1, b2, b3, b4, b5, b6, output p::req_t r1, r2);
  import p::*;
  localparam req_t LOC = '{{addr: 8'hEE, st: RUN, v: 1'b0}};
  req_t vr = '{{addr: 8'h11, st: IDLE, v: 1'b1}};
  always_comb b1 = '{{rq: '{{default: RST_REQ}}, tag: a}};
  always_comb b2 = '{{rq: '{{default: LOC}}, tag: a}};
  initial b3 = '{{rq: '{{default: RST_REQ}}, tag: 4'h3}};
  always_comb b4 = '{{rq: '{{RST_REQ, LOC}}, tag: a}};
  initial b5 = '{{rq: '{{RST_REQ, LOC}}, tag: 4'h3}};
  always_comb b6 = '{{rq: '{{vr, vr}}, tag: a}};
  assign r1 = RST_REQ;
  assign r2 = LOC;
endmodule
module top;
  import p::*;
  logic [3:0] a = 4'h3;
  bundle_t b1, b2, b3, b4, b5, b6, t1, t2; req_t r1, r2;
  localparam req_t LOC = '{{addr: 8'hEE, st: RUN, v: 1'b0}};
  child c(.a(a), .b1(b1), .b2(b2), .b3(b3), .b4(b4), .b5(b5), .b6(b6), .r1(r1), .r2(r2));
  always_comb t1 = '{{rq: '{{default: RST_REQ}}, tag: a}};
  initial t2 = '{{rq: '{{default: LOC}}, tag: 4'h3}};
  initial begin
    #1 $display(\"T|child b1=%h b2=%h b3=%h b4=%h b5=%h b6=%h r1=%h r2=%h\", b1, b2, b3, b4, b5, b6, r1, r2);
    $display(\"T|top t1=%h t2=%h\", t1, t2);
  end
endmodule
"
    ));
    assert_eq!(
        out,
        [
            "T|child b1=3feffd3 b2=3b97723 b3=3feffd3 b4=3fef723 b5=3fef723 b6=0448893 r1=7fd r2=772",
            "T|top t1=3feffd3 t2=3b97723",
        ]
    );
}
