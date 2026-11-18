-- IEEE 1076-2008 14.7.5.2: ordinary processes precede postponed ones.
entity initialization_batches is end;
architecture test of initialization_batches is
begin
  workers: for i in 0 to 1024 generate
    worker: process
    begin
      if i = 0 then
        report "first ordinary";
      elsif i = 1023 then
        report "ordinary at batch boundary";
      elsif i = 1024 then
        report "last ordinary";
      end if;
      wait;
    end process;
  end generate;
  observer: postponed process
  begin
    report "postponed after ordinary";
    wait;
  end process;
  completion: process
  begin
    wait for 1 ns;
    report "PASS initialization batches";
    std.env.finish;
  end process;
end;
