-- IEEE 1076-2008 §4.2.2.3: procedure writes belong to the calling driver.
entity pending_procedure_delay is end;
architecture a of pending_procedure_delay is
  signal q : bit := '0';
  procedure delayed is begin q <= '0' after 1 ns; end;
  procedure nested is begin delayed; end;
begin
  process
  begin
    q <= '1';
    wait for 0 ns;
    assert q = '1' report "immediate transaction lost" severity failure;
    nested;
    wait for 1 ns;
    assert q = '0' report "procedure future transaction lost" severity failure;
    report "PASS pending procedure delay";
    std.env.finish;
  end process;
end;
