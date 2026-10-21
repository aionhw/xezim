module spi();
  specparam delay = 50;
  
  // Invalid: specparam used in parameter expression
  parameter p = delay + 2;  // Should be error
  
  logic a;
endmodule