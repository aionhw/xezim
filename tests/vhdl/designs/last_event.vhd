entity last_event is end;
architecture rtl of last_event is
  signal q : bit := '0';
  function age(signal x : bit) return time is begin return x'last_event; end;
  function nested(signal x : bit) return time is begin return age(x); end;
begin
  process begin
    wait for 1 ns; assert nested(q) = time'high severity failure; q <= '1'; wait for 0 ns;
    assert nested(q) = 0 ns severity failure;
    wait for 3 ns; q <= '1'; wait for 0 ns;
    assert nested(q) = 3 ns severity failure;
    q <= '0'; wait for 0 ns;
    assert q'last_event = 0 ns severity failure;
    report "PASS event times"; std.env.finish; wait;
  end process;
end;
