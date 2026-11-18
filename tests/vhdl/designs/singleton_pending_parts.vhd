-- IEEE 1076-2008 §10.5.2, §14.7.2, §16.2.4: scalar pending transactions.
library ieee;
use ieee.std_logic_1164.all;
entity singleton_pending_parts is end;
architecture a of singleton_pending_parts is
  signal q : bit_vector(2 downto 0) := "000";
  type words is array (0 to 1) of bit_vector(3 downto 0);
  signal w : words := (others => "0000");
  type row is record value : bit_vector(0 to 0); keep : bit; end record;
  signal v : row := (value => "0", keep => '0');
  signal s : std_logic_vector(1 downto 0) := "00";
begin
  process
  begin
    q(0) <= '0'; q(0) <= '0'; q(0) <= '0';
    w(1)(2) <= '1';
    v.value <= "1"; v.value(0) <= '0'; v.value(0 to 0) <= "1";
    s(0) <= '0';
    wait for 0 ns;
    assert q = "010" and not q(0)'event and q(1)'event
      report "scalar pending values or events changed" severity failure;
    assert q(0)'active and q(1)'active and not q(2)'active and q(2)'quiet(1 ns)
      report "equal writes lost activity or touched siblings" severity failure;
    assert q(0)'driving and q(0)'driving_value = '0'
      report "selected driver value changed before its update" severity failure;
    assert w(1) = "0100" and w(0)'quiet(1 ns) and w(1)(3)'quiet(1 ns)
      report "nested scalar write touched another leaf" severity failure;
    assert v.value = "1" and v.value(0)'active and v.keep'quiet(1 ns)
      report "field and slice precedence changed" severity failure;
    assert s(0) = '0' and not s(0)'event and s(0)'active and s(1)'quiet(1 ns)
      report "resolved equal writes lost scalar activity" severity failure;
    q(0) <= '0'; q(0) <= '1'; q(0) <= '1';
    v.value(0) <= '0'; v.value <= "1";
    wait for 0 ns;
    assert q = "011" and q(0)'event and q(0)'active and not q(1)'active
      report "repeated scalar replacement changed" severity failure;
    assert q(0)'driving_value = '1'
      report "selected driver did not retain the applied scalar" severity failure;
    assert v.value = "1" and not v.value(0)'event and v.value(0)'active
      report "leaf before whole field lost precedence or activity" severity failure;
    assert s(0) = 'X' and s(0)'event and s(0)'active and not s(1)'active
      report "separate singleton drivers lost resolution" severity failure;
    wait for 1 ns;
    assert q(0)'quiet(1 ns) and q(1)'quiet(1 ns) and q(2)'quiet(1 ns)
      report "singleton quiet deadline changed" severity failure;
    report "PASS singleton pending parts";
    std.env.finish;
  end process;
  process
  begin
    q(1) <= '1';
    wait;
  end process;
  process
  begin
    s(0) <= '0';
    wait for 0 ns;
    s(0) <= '1';
    wait;
  end process;
end;
