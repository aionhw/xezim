-- Postponed processes (§11.3, §14.7.5.3): they run at the end of a time
-- step and see its final values, and at initialization after the other
-- processes; a postponed concurrent assertion and procedure call, and a
-- postponed process resumed by a timeout, and one whose wait condition
-- holds when it resumes but not at the end of the time step, where it runs
-- and evaluates the condition (§10.2), so it suspends again.
entity postponed_procs is end entity;
architecture a of postponed_procs is
  signal a, b, c, e, f : integer := 0;
  procedure show(signal x : in integer; name : string) is
  begin
    report name & "=" & integer'image(x);
  end procedure;
begin
  process
  begin
    a <= 1; wait for 10 ns;
    a <= 2; wait for 10 ns;
    a <= 3; a <= 4 after 1 ns; wait for 10 ns;
    for i in 1 to 2 loop
      e <= i; wait for 0 ns; e <= i * 10; wait for 0 ns; e <= i * 100;
      wait for 10 ns;
    end loop;
    f <= 1; wait for 0 ns; f <= 0;
    wait;
  end process;
  b <= a + 1;
  c <= b * 2;
  postponed process (a, b, c) begin
    report "a=" & integer'image(a) & " b=" & integer'image(b) & " c=" & integer'image(c);
  end process;
  postponed assert c /= 6 report "c is 6" severity note;
  postponed show(e, "e");
  postponed process
  begin
    wait for 45 ns;
    report "timeout at " & time'image(now) & " e=" & integer'image(e);
    wait;
  end process;
  postponed process
  begin
    wait until f = 1;
    report "f was 1, now f=" & integer'image(f) & " at " & time'image(now);
    wait;
  end process;
end architecture;
