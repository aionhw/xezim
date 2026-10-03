// Pure-SV self-test: a type-parameter DEFAULT that references a SIBLING
// type parameter (IEEE 1800-2023 8.25.4: `class C #(type REQ=int, type
// RSP=REQ)`) must bind, at instantiation of `C#(simple_item)`, RSP to the
// CONCRETE sibling argument (simple_item), not the literal name "REQ".
// The UVM `uvm_declare_p_sequencer` cast depends on this: $cast to a
// member of declared type `C#(simple_item)` compares the instance's
// type_bindings; an unresolved "REQ" binding made every such cast fail.
class item_a;
endclass

class sqr_base;
  int x;
  function new(); x = 7; endfunction
endclass

class sqr_t #(type REQ = int, type RSP = REQ) extends sqr_base;
  REQ q0;
  RSP r0;
endclass

class other extends sqr_base;
  function new(); x = 9; endfunction
endclass

class base_seq;
  sqr_base m_sequencer;
  virtual function void m_set_p_sequencer(); return; endfunction
  virtual function void set_sequencer(sqr_base s);
    m_sequencer = s;
    m_set_p_sequencer();
  endfunction
endclass

class sub_seq extends base_seq;
  sqr_t#(item_a) p_sequencer; // explicit specialization member (macro shape)
  virtual function void m_set_p_sequencer();
    super.m_set_p_sequencer();
    if (!$cast(p_sequencer, m_sequencer)) begin
      $display("TAG_FAIL cast failed");
      return;
    end
  endfunction
endclass

module top;
  initial begin
    automatic sqr_t#(item_a) s = new();   // RSP defaults to REQ = item_a
    automatic other o = new();            // wrong type for the cast
    automatic sub_seq sq = new();
    automatic item_a ia = new();
    automatic int ok = 0;

    sq.set_sequencer(s);                  // base-class virtual dispatch casts
    if (sq.p_sequencer != null && sq.p_sequencer.x == 7) ok++;

    // The default binding must also be USABLE: RSP (= item_a) must accept
    // an item_a instance...
    if ($cast(sq.p_sequencer.r0, ia)) ok++; else $display("r0 cast of item_a failed");
    // ...and reject a non-item_a source.
    if (!$cast(sq.p_sequencer.r0, s)) ok++; else $display("r0 cast of sqr_t wrongly succeeded");

    // A genuinely different type must still fail the cast
    sq.m_sequencer = o;
    if (!$cast(sq.p_sequencer, sq.m_sequencer)) ok++;
    else $display("cast of other should have failed");

    if (ok == 4) $display("TAG_PASS");
    else $display("TAG_FAIL ok=%0d", ok);
  end
endmodule
