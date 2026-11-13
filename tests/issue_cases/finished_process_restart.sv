// IEEE 1800-2023 §9.3.2 / Annex D.9: restore a previously finished child.
module top;
  int fd;
  initial begin
    fork
      #10 if ($fopen("child.flag", "r") != 0) forever #0;
    join_none
    #5 $save("child.ckpt");
    #10;
    // After restore this parent must park, leaving the revived child's
    // zero-delay loop as the only runnable work. This remains a fatal stall
    // when restoration correctly preserves the parent's entire tail.
    if ($fopen("child.flag", "r") != 0) wait (0);
    fd = $fopen("child.flag", "w"); $fclose(fd);
    $restart("child.ckpt");
    // Checkpoint operations commit when the slot advances.
    #1;
    wait (0);
  end
endmodule
