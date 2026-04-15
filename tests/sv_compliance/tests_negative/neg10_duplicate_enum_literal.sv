// EXPECT: compile_fail
module neg10_duplicate_enum_literal;
  typedef enum logic [1:0] {
    IDLE = 2'd0,
    RUN  = 2'd1,
    IDLE = 2'd2
  } state_t;
endmodule
