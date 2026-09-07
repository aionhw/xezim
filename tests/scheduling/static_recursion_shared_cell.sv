// Pure-SV regression for §6.21/§13.3.1: a `static` local in a method is ONE
// live storage cell shared across ALL simultaneous activations — including
// re-entrant/recursive calls. Previously, xezim kept a per-recursion-frame
// copy restored at entry and written back on return, so an inner recursion
// level's mutation of the shared cell was invisible to the caller frames and
// the static was effectively reset to its initializer on each re-entry
// ("fresh frame"). This caused UVM `phase.jump` re-entry to re-run
// `main_phase` with its `static bit first` re-initialised to 1,
// livelocking the phase scheduler.
//
// A recursive method that accumulates into a `static int` must see the whole
// sequence (depth 4,3,2,1 -> 4 increments -> shared counter == 4), matching the
// reference simulator byte-for-byte:
//   TAG_INNER4 depth=4 counter=1 saved=1
//   TAG_OUTER4 depth=4 counter=4 saved=1
//   TAG_PASS counter=4
module top;
  class C;
    int r;
    function int recurse(int level, int depth);
      static int counter = 0;
      int saved;
      if (depth <= 0) return counter;
      counter += 1;
      saved = counter;
      recurse(level + 1, depth - 1);
      return counter;
    endfunction
  endclass

  C c;
  int r;
  initial begin
    c = new();
    r = c.recurse(0, 4);
    // Reference/expected: because `counter` is a single shared cell across the
    // recursion, the outermost activation's read after all 4 increments is 4.
    if (r == 4) $display("TAG_PASS counter=%0d", r);
    else $display("TAG_FAIL counter=%0d (expected shared accumulation across recursion)", r);
  end
endmodule