// delete(index) removes the entry with that index from an associative array
// (IEEE 1800-2017 7.9.2), however the entry was written. Here the arrays are
// class properties with struct elements, written either from an assignment
// pattern (10.9) or from a struct variable; after delete(), num() is 0 and
// exists() is 0. Module-level arrays are checked alongside as a reference.
typedef struct {
    int lo;
    int hi;
} span_t;

typedef struct packed {
    logic [15:0] lo;
    logic [15:0] hi;
} pspan_t;

class holder;
    span_t  by_long[longint unsigned];
    span_t  by_int[int];
    span_t  by_str[string];
    pspan_t packed_el[longint unsigned];

    function void put_named(longint unsigned k); by_long[k] = '{lo: 1, hi: 2}; endfunction
    function void put_positional(longint unsigned k); by_long[k] = '{1, 2}; endfunction
    function void put_var(longint unsigned k);
        span_t v = '{lo: 1, hi: 2};
        by_long[k] = v;
    endfunction
    function void del(longint unsigned k); by_long.delete(k); endfunction
    // the shape a lookup table uses: collect matching keys, then delete them
    function void del_matching(int at);
        longint unsigned hit[$];
        foreach (by_long[i]) if (by_long[i].lo <= at && at <= by_long[i].hi) hit.push_back(i);
        foreach (hit[i]) by_long.delete(hit[i]);
    endfunction
endclass

module top;
    span_t m_by_long[longint unsigned];
    int    fails = 0;

    task automatic check(string what, int got, int exp);
        if (got != exp) begin
            fails++;
            $display("FAIL %-46s got %0d, expected %0d", what, got, exp);
        end
    endtask

    initial begin
        holder h = new;

        // reference: module-level array, pattern store
        m_by_long[9] = '{lo: 1, hi: 2};
        m_by_long.delete(9);
        check("module array, named pattern: num()", m_by_long.num(), 0);

        // class property, written through a variable
        h.put_var(9);
        h.del(9);
        check("class, variable store, delete in method", h.by_long.num(), 0);

        // class property, written from an assignment pattern
        h.put_named(9);
        h.del(9);
        check("class, named pattern, delete in method", h.by_long.num(), 0);
        check("  exists() after delete", h.by_long.exists(9), 0);
        h.by_long.delete();

        h.put_positional(9);
        h.del(9);
        check("class, positional pattern, delete in method", h.by_long.num(), 0);
        h.by_long.delete();

        h.by_long[9] = '{lo: 1, hi: 2};
        h.by_long.delete(9);
        check("class, pattern and delete from outside", h.by_long.num(), 0);
        h.by_long.delete();

        h.put_named(9);
        h.del_matching(1);
        check("class, pattern, delete of collected keys", h.by_long.num(), 0);
        h.by_long.delete();

        // other index and element types
        h.by_int[9] = '{lo: 1, hi: 2};
        h.by_int.delete(9);
        check("class, int index", h.by_int.num(), 0);

        h.by_str["a"] = '{lo: 1, hi: 2};
        h.by_str.delete("a");
        check("class, string index", h.by_str.num(), 0);

        h.packed_el[9] = '{lo: 16'd1, hi: 16'd2};
        h.packed_el.delete(9);
        check("class, packed struct element", h.packed_el.num(), 0);

        // one pattern-written entry among others: only that one goes
        h.put_var(1);
        h.put_named(2);
        h.put_var(3);
        h.del(2);
        check("class, mixed entries, delete the pattern one", h.by_long.num(), 2);
        check("  the deleted index is gone", h.by_long.exists(2), 0);

        $display("%s", fails ? "TEST FAILED" : "TEST PASSED");
        $finish;
    end
endmodule
