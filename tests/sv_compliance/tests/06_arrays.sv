`include "../common/svtest_defs.svh"

module test_arrays;
  `SVTEST_INIT

  bit [7:0] packed_arr;
  int unpacked_arr [0:3];
  int dyn_arr [];
  int q [$];
  int aa [string];

  initial begin
    packed_arr = 8'hA6;

    unpacked_arr[0] = 3;
    unpacked_arr[1] = 5;
    unpacked_arr[2] = 7;
    unpacked_arr[3] = 9;

    dyn_arr = new[3];
    dyn_arr[0] = 11;
    dyn_arr[1] = 13;
    dyn_arr[2] = 17;

    q.push_back(21);
    q.push_back(22);
    q.push_front(20);

    aa["alpha"] = 31;
    aa["beta"] = 37;

    `SVTEST_CHECK(packed_arr[7:4] == 4'hA, "packed array slice failed")
    `SVTEST_CHECK(unpacked_arr[2] == 7, "unpacked array indexing failed")
    `SVTEST_CHECK(dyn_arr.size() == 3 && dyn_arr[1] == 13, "dynamic array failed")
    `SVTEST_CHECK(q.size() == 3 && q[0] == 20 && q[2] == 22, "queue operation failed")
    `SVTEST_CHECK(aa.exists("alpha") && aa["beta"] == 37, "associative array failed")

    `SVTEST_PASSFAIL
  end
endmodule
