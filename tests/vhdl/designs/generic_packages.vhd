-- Generic packages and their instantiation (§4.9, §6.5.6): a generic type,
-- constants and a function (bound, not called: GHDL 4.1 cannot call one),
-- instances as a library unit and in an architecture, each with its own
-- types and subprograms.
package fifo_pkg is
  generic (type elem; depth : positive; init : elem; function dflt return elem);
  type store is array (0 to depth - 1) of elem;
  function first(s : store) return elem;
  procedure fill(s : inout store; v : elem);
  function empty return store;
  constant size : natural := depth;
end package;
package body fifo_pkg is
  function first(s : store) return elem is begin return s(0); end function;
  procedure fill(s : inout store; v : elem) is
  begin for i in s'range loop s(i) := v; end loop; end procedure;
  function empty return store is
    variable r : store;
  begin
    for i in r'range loop r(i) := init; end loop;
    return r;
  end function;
end package body;

package ints is
  function seven return integer;
end package;
package body ints is
  function seven return integer is begin return 7; end function;
end package body;

package int_fifo is new work.fifo_pkg generic map (elem => integer, depth => 4, init => 7, dflt => work.ints.seven);

use work.int_fifo.all;
entity generic_packages is end entity;
architecture a of generic_packages is
  function bdflt return bit is begin return '1'; end function;
  package bit_fifo is new work.fifo_pkg generic map (elem => bit, depth => 2, init => '1', dflt => bdflt);
begin
  process
    variable s : store := (others => 5);
    variable b : bit_fifo.store;
  begin
    report "first=" & integer'image(first(s)) & " size=" & integer'image(size);
    fill(s, 9);
    report "first=" & integer'image(first(s));
    s := empty;
    report "empty first=" & integer'image(first(s));
    b := bit_fifo.empty;
    report "bit first=" & bit'image(bit_fifo.first(b)) & " size=" & integer'image(bit_fifo.size);
    bit_fifo.fill(b, '0');
    report "bit first=" & bit'image(bit_fifo.first(b));
    wait;
  end process;
end architecture;
