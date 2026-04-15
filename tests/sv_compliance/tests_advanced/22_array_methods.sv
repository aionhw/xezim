`include "../common/svtest_defs.svh"

module test_array_methods;
  `SVTEST_INIT

  int q[$];
  int da[];
  int aa[string];
  int popped;
  int total;

  initial begin
    q = {3, 1, 2};
    q.sort();
    `SVTEST_CHECK(q.size() == 3, "queue size() failed")
    `SVTEST_CHECK(q[0] == 1 && q[1] == 2 && q[2] == 3, "queue sort() failed")

    q.push_back(4);
    popped = q.pop_front();
    `SVTEST_CHECK(popped == 1, "queue pop_front() failed")
    `SVTEST_CHECK(q.size() == 3, "queue push_back()/pop_front() size mismatch")

    da = new[3];
    da[0] = 4;
    da[1] = 5;
    da[2] = 6;
    total = da.sum();
    `SVTEST_CHECK(total == 15, "dynamic array sum() failed")

    aa["alice"] = 10;
    aa["bob"]   = 20;
    `SVTEST_CHECK(aa.num() == 2, "associative array num() failed")
    `SVTEST_CHECK(aa.exists("alice"), "associative array exists() failed")
    `SVTEST_CHECK(aa["bob"] == 20, "associative array indexing failed")

    `SVTEST_PASSFAIL
  end
endmodule
