// Ch.21.3+ file I/O
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[21] %s", name); fails++; end
  initial begin
    int fd, code, v1, v2;
    string line, s;
    fd = $fopen("/tmp/xz_audit_io.txt", "w");
    `CK("fopen w", fd != 0)
    $fwrite(fd, "12 34\n");
    $fdisplay(fd, "line2 %0d", 99);
    $fclose(fd);
    fd = $fopen("/tmp/xz_audit_io.txt", "r");
    `CK("fopen r", fd != 0)
    code = $fscanf(fd, "%d %d", v1, v2);
    `CK("fscanf count", code == 2)
    `CK("fscanf values", v1 == 12 && v2 == 34)
    code = $fgets(line, fd);
    `CK("fgets rest of line", code >= 0)
    code = $fgets(line, fd);
    `CK("fgets line2", line == "line2 99\n" || line == "line2 99")
    `CK("feof", $feof(fd) != 0 || $fgets(line, fd) == 0)
    $fclose(fd);
    begin // $sscanf
      int a, b;
      code = $sscanf("7 8", "%d %d", a, b);
      `CK("sscanf", code == 2 && a == 7 && b == 8)
      code = $sscanf("hex ff", "hex %h", a);
      `CK("sscanf hex", code == 1 && a == 255)
    end
    begin // $readmemh
      logic [7:0] mem[0:3];
      fd = $fopen("/tmp/xz_audit_mem.hex", "w");
      $fdisplay(fd, "AA BB CC DD");
      $fclose(fd);
      $readmemh("/tmp/xz_audit_mem.hex", mem);
      `CK("readmemh", mem[0] == 8'hAA && mem[3] == 8'hDD)
    end
    $display("CH21 CHECKS DONE fails=%0d", fails);
  end
endmodule
