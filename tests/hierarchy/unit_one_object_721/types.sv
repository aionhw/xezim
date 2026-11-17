typedef enum {RED, GREEN, BLUE} color_t;
typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pk_t;
typedef struct { int a; string s; } up_t;
class Box; int v; function new(int x); v = x; endfunction endclass
bit        u_bit = 1;
logic [15:0] u_vec = 16'h1234;
real       u_real = 1.5;
string     u_str = "s0";
color_t    u_enum = GREEN;
pk_t       u_pk = 8'ha5;
up_t       u_up = '{a: 3, s: "u"};
int        u_fix [3] = '{1, 2, 3};
int        u_dyn [];
int        u_aa [string];
int        u_q [$] = {7, 8};
Box        u_box;
event      u_ev;
int        u_base = 10;
int        u_expr = u_base * 2 + 1;
function automatic int ufn(int k); return k + u_base; endfunction
int        u_call = ufn(5);
module writer;
  initial begin
    #1;
    u_bit = 0; u_vec[7:0] = 8'hff; u_real = 2.25; u_str = {u_str, "w"}; u_enum = BLUE;
    u_pk.lo = 4'h3; u_up.a = 9; u_up.s = "w"; u_fix[1] = 20; u_dyn = new[2]; u_dyn[1] = 5;
    u_aa["k"] = 4; u_q.push_back(9); u_box = new(11); u_expr++;
    #1; -> u_ev;
  end
endmodule
module reader #(parameter int ID = 0);
  initial begin
    #2;
    $display("T|r%0d %b %h %0.2f %s %s %h %0d %s", ID, u_bit, u_vec, u_real, u_str, u_enum.name(), u_pk, u_up.a, u_up.s);
    $display("T|r%0d %0d %0d %0d %0d %0d %0d %0d %0d %0d", ID, u_fix[1], u_dyn.size(), u_dyn[1], u_aa["k"], u_aa.num(), u_q.size(), u_q[2], u_box.v, u_expr);
    $display("T|r%0d init %0d %0d", ID, u_call, $unit::u_call);
  end
  initial begin @(u_ev); $display("T|r%0d event at %0t", ID, $time); end
  initial begin wait (u_q.size() == 3); $display("T|r%0d wait at %0t", ID, $time); end
  always @(u_vec) $display("T|r%0d vec %h", ID, u_vec);
  always_comb if (u_enum == BLUE) $display("T|r%0d comb blue", ID);
endmodule
module mid; reader #(2) r(); endmodule
module top;
  writer w(); reader #(1) r(); mid m();
  wire [15:0] cw = u_vec;
  initial begin
    $monitor("T|mon %0d %h", u_fix[1], cw);
    #3; foreach (u_fix[i]) $display("T|top fix[%0d]=%0d", i, u_fix[i]);
    $display("T|top %s %0d %p", u_str, u_q.pop_front(), u_q);
    #1 $finish;
  end
endmodule
