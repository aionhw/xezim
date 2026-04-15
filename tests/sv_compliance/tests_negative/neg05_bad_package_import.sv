// EXPECT: compile_fail
package neg05_pkg;
  typedef int good_t;
endpackage

module neg05_bad_package_import;
  import neg05_pkg::missing_t;
  missing_t x;
endmodule
