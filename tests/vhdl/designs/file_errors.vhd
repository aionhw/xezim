-- A READ of a file that is not open stops the simulation (§5.5.2) with
-- GHDL's message; FILE_CLOSE of a closed file does nothing.
entity file_errors is
end entity;

architecture a of file_errors is
  type int_file is file of integer;
begin
  process
    file f : int_file;
    variable v : integer;
  begin
    file_close(f);
    file_open(f, "data.bin", write_mode);
    write(f, 5);
    file_close(f);
    file_close(f);
    report "closed twice";
    read(f, v);
    report "not reached";
    wait;
  end process;
end architecture;
