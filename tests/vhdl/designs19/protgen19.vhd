-- VHDL-2019 §5.6.2, §5.6.4 generic protected types: a stack with a type
-- generic and a constant generic, two instances (type ... is new ...
-- generic map), the body seeing the generics and declaring a type of
-- them. nvc 1.23.1 has no generic protected types, so the golden is
-- written from the LRM (protgen19.lrm).
use std.textio.all;
package stack_pkg is
  type stack_pt is protected
    generic (type elem; depth : positive);
    procedure push(x : elem);
    impure function pop return elem;
    impure function size return natural;
  end protected;
  type int_stack is new stack_pt generic map (elem => integer, depth => 4);
  type bit_stack is new stack_pt generic map (elem => bit, depth => 2);
end package;

package body stack_pkg is
  type stack_pt is protected body
    type store_t is array (1 to depth) of elem;
    variable store : store_t;
    variable n : natural := 0;
    procedure push(x : elem) is
    begin
      assert n < depth report "stack full" severity failure;
      n := n + 1;
      store(n) := x;
    end procedure;
    impure function pop return elem is
    begin
      n := n - 1;
      return store(n + 1);
    end function;
    impure function size return natural is
    begin
      return n;
    end function;
  end protected body;
end package body;

use std.textio.all;
use work.stack_pkg.all;
entity protgen19 is end entity;
architecture a of protgen19 is
  shared variable s : int_stack;
  shared variable b : bit_stack;
begin
  process
    variable l : line;
  begin
    s.push(1);
    s.push(2);
    s.push(3);
    b.push('1');
    b.push('0');
    write(l, integer'image(s.pop) & integer'image(s.pop) & " " & integer'image(s.size)
      & " " & bit'image(b.pop) & integer'image(b.size));
    writeline(output, l);
    b.push('1');
    b.push('1');
    wait;
  end process;
end architecture;
