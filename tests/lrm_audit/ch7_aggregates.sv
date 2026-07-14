// IEEE 1800-2017 Ch.7 — aggregate types
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[7] %s", name); fails++; end
  initial begin
    begin // 7.2 structs packed/unpacked
      typedef struct packed { logic [3:0] hi; logic [3:0] lo; } p_t;
      typedef struct { int a; string s; } u_t;
      p_t p; u_t u1, u2;
      p = 8'hA5;
      `CK("packed struct slice", p.hi == 4'hA && p.lo == 4'h5)
      p.lo = 4'hF;
      `CK("packed member write", p == 8'hAF)
      u1.a = 3; u1.s = "hi";
      u2 = u1;
      `CK("unpacked struct copy", u2.a == 3 && u2.s == "hi")
    end
    begin // 7.3 unions
      typedef union packed { logic [7:0] b; logic [1:0][3:0] n; } pu_t;
      pu_t pu;
      pu.b = 8'h5A;
      `CK("packed union alias", pu.n[1] == 4'h5 && pu.n[0] == 4'hA)
    end
    begin // 7.4 packed multi-d
      logic [1:0][3:0] m;
      m = 8'hC3;
      `CK("packed 2d index", m[1] == 4'hC && m[0] == 4'h3)
      m[0][1] = 1'b0;
      `CK("packed 2d bit write", m == 8'hC1)
    end
    begin // 7.5 dynamic arrays
      int d[]; int e[];
      d = new[4];
      `CK("new size", d.size() == 4)
      foreach (d[i]) d[i] = i * i;
      e = new[6](d);
      `CK("new copy", e[3] == 9 && e.size() == 6)
      d.delete();
      `CK("delete", d.size() == 0)
    end
    begin // 7.8 associative arrays
      int aa[string]; int ia[int]; string k;
      aa["one"] = 1; aa["two"] = 2;
      `CK("assoc num", aa.num() == 2)
      `CK("assoc exists", aa.exists("one") == 1)
      `CK("assoc read", aa["two"] == 2)
      `CK("first", aa.first(k) == 1 && k == "one")
      `CK("next", aa.next(k) == 1 && k == "two")
      aa.delete("one");
      `CK("delete key", aa.num() == 1 && !aa.exists("one"))
      ia[100] = 7; ia[-5] = 3;
      begin int ik; ia.first(ik); `CK("int-key order", ik == -5) end
    end
    begin // 7.10 queues
      int q[$]; int r;
      q.push_back(1); q.push_back(2); q.push_front(0);
      `CK("queue order", q[0] == 0 && q[2] == 2)
      `CK("$ index", q[$] == 2)
      r = q.pop_front();
      `CK("pop_front", r == 0 && q.size() == 2)
      q.insert(1, 99);
      `CK("insert", q[1] == 99 && q.size() == 3)
      q.delete(1);
      `CK("delete idx", q[1] == 2)
      q = {};
      `CK("clear", q.size() == 0)
    end
    begin // 7.12 array methods
      int a[5] = '{4, 1, 3, 5, 2};
      int q[$], idx[$];
      `CK("sum", a.sum() == 15)
      `CK("min", a.min() == '{1})
      `CK("max", a.max() == '{5})
      q = a.find with (item > 2);
      `CK("find", q.size() == 3)
      idx = a.find_first_index with (item == 3);
      `CK("find_first_index", idx.size() == 1 && idx[0] == 2)
      begin
        int s[5];
        s = a;
        s.sort();
        `CK("sort", s[0] == 1 && s[4] == 5)
        s.rsort();
        `CK("rsort", s[0] == 5 && s[4] == 1)
        s.reverse();
        `CK("reverse", s[0] == 1)
      end
      q = a.unique();
      `CK("unique count", q.size() == 5)
    end
    $display("CH7 CHECKS DONE fails=%0d", fails);
  end
endmodule
