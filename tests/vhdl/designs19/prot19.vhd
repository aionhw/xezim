-- VHDL-2019 protected types (§5.6.2): private variables, of a scalar type
-- and of another protected type, used by the body and published by an
-- alias of the declaration.
use std.textio.all;
package prot_pkg is
  type counter_t is protected
    procedure inc(n : integer);
    impure function get return integer;
  end protected counter_t;
  type pair_t is protected
    private variable lo : counter_t;
    private variable steps : integer := 100;
    alias low is lo.get [return integer];
    procedure bump;
    impure function total return integer;
  end protected pair_t;
end package;

package body prot_pkg is
  type counter_t is protected body
    variable count : integer := 0;
    procedure inc(n : integer) is
    begin
      count := count + n;
    end procedure;
    impure function get return integer is
    begin
      return count;
    end function;
  end protected body counter_t;

  type pair_t is protected body
    variable hi : counter_t;
    procedure bump is
    begin
      lo.inc(1);
      hi.inc(10);
      steps := steps + 1;
    end procedure;
    impure function total return integer is
    begin
      return lo.get + hi.get + steps;
    end function;
  end protected body pair_t;
end package body;

use std.textio.all;
use work.prot_pkg.all;
entity prot19 is end entity;
architecture a of prot19 is
  shared variable p, q : pair_t;
begin
  process
    variable l : line;
  begin
    for i in 1 to 3 loop
      p.bump;
    end loop;
    q.bump;
    write(l, "p.low=" & integer'image(p.low) & " p.total=" & integer'image(p.total)
      & " q.low=" & integer'image(q.low) & " q.total=" & integer'image(q.total));
    writeline(output, l);
    wait;
  end process;
end architecture;
