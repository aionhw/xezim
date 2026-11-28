`timescale 1ns/100ps
module top;
  int fd, rc, delim;
  real first, second;
  string suffix;

  initial begin
    // §21.3.1, §21.3.4.3 and §21.3.8: stream matching, pushback and EOF.
    fd = $fopen("scan-time.txt", "w");
    if (fd == 0) $fatal(1, "open for writing");
    $fwrite(fd, "1.25!\n10.345 ms\n-10.345X\n2.5e-V\n1.005");
    $fclose(fd);
    fd = $fopen("scan-time.txt", "r");
    if (fd == 0) $fatal(1, "open for reading");
    rc = $fscanf(fd, "%f", first);
    delim = $fgetc(fd);
    if (rc != 1 || first != 1.25 || delim != "!") $fatal(1, "float file guard");

    $timeformat(-3, 2, " ms", 10);
    rc = $fscanf(fd, "%t %s %t", first, suffix, second);
    if (rc != 3 || first != 10350000.0 || suffix != "ms" || second != -10350000.0)
      $fatal(1, "time fields across lines: rc=%0d first=%f second=%f", rc, first, second);
    delim = $fgetc(fd);
    if (delim != "X" || $feof(fd)) $fatal(1, "delimiter left unread");
    rc = $fscanf(fd, "%t", first);
    delim = $fgetc(fd);
    if (rc != 0 || first != 10350000.0 || delim != "V") $fatal(1, "incomplete exponent pushback");
    $fscanf(fd, "%t", second);
    if (second != 1010000.0) $fatal(1, "task-form fscanf");
    rc = $fscanf(fd, "%t", second);
    if (rc != -1 || !$feof(fd) || second != 1010000.0) $fatal(1, "time field at EOF");
    rc = $fscanf(fd, "%g", second);
    if (rc != -1 || second != 1010000.0) $fatal(1, "float field at EOF");
    $fclose(fd);
    $display("PASS");
  end
endmodule
