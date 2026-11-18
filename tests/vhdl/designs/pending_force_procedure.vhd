-- IEEE 1076-2008 §10.5.2.1, §14.7.3.1: force does not stop driver updates.
entity pending_force_procedure is end;
architecture a of pending_force_procedure is
  signal q : bit := '0';
  procedure force_q is begin q <= force '1'; end;
begin
  process
  begin
    q <= '0';
    wait for 1 ns;
    q <= '1';
    wait for 1 ns;
    q <= '0';
    wait;
  end process;
  process
  begin
    wait for 0 ns;
    force_q;
    wait for 0 ns;
    assert q = '1' report "procedure force lost" severity failure;
    wait for 1 ns;
    assert q = '1' report "forced value changed" severity failure;
    q <= release;
    wait for 0 ns;
    assert q = '1' report "release lost current driver" severity failure;
    wait for 1 ns;
    wait for 0 ns;
    assert q = '0' report "released driver update lost" severity failure;
    report "PASS pending force procedure";
    std.env.finish;
  end process;
end;
