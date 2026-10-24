-- Signal and scalar attributes (§16.2): 'event, 'last_value, 'last_event,
-- 'active, type attributes of enumeration and integer subtypes.
entity attrs is end entity;
architecture a of attrs is
  type color is (red, green, blue, yellow);
  subtype warm is color range red to green;
  subtype small is integer range -3 to 7;
  signal s : bit := '0';
  signal c : color := red;
begin
  process
  begin
    report "color'left=" & color'image(color'left) & " right=" & color'image(color'right)
      & " pos(blue)=" & integer'image(color'pos(blue)) & " val(3)=" & color'image(color'val(3))
      & " succ(red)=" & color'image(color'succ(red)) & " pred(yellow)=" & color'image(color'pred(yellow));
    report "warm'high=" & color'image(warm'high) & " small'low=" & integer'image(small'low)
      & " small'high=" & integer'image(small'high);
    wait for 4 ns;
    s <= '1';
    c <= blue;
    wait for 0 ns;
    report "s'event=" & boolean'image(s'event) & " s'last_value=" & bit'image(s'last_value)
      & " c'last_value=" & color'image(c'last_value) & " s'active=" & boolean'image(s'active);
    wait for 6 ns;
    report "s'event=" & boolean'image(s'event) & " s'last_event=" & time'image(s'last_event);
    s <= '1';
    wait for 0 ns;
    report "same value: s'event=" & boolean'image(s'event) & " s'active=" & boolean'image(s'active);
    wait;
  end process;
end architecture;
