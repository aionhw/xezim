// IEEE 1800-2023 §9.3.2 / Annex D.9: restore a previously finished child.
module top;
  int fd;
  initial begin
    fork
      #10 if ($fopen("child.flag", "r") != 0) forever #0;
    join_none
    #5 $save("child.ckpt");
    #10;
    fd = $fopen("child.flag", "w"); $fclose(fd);
    $restart("child.ckpt");
    #5 $finish;
  end
endmodule
