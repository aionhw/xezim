-- Protected types (§5.6): a declaration in a package and its body in the
-- package body, a shared variable used from two entities and from a
-- clocked process, variables of protected types in a process (each its own
-- object), a protected type whose body holds an access-type queue and an
-- object of another protected type, private subprograms, methods calling
-- each other without a prefix, functions returning strings, and an object
-- passed to a procedure.
package sb_pkg is
  type counter_t is protected
    procedure inc(by : integer := 1);
    impure function get return integer;
    procedure reset;
  end protected;
  type queue_t is protected
    procedure push(v : integer);
    impure function pop return integer;
    impure function empty return boolean;
    impure function size return natural;
    impure function describe return string;
    procedure push_all(a : integer_vector);
  end protected;
end package;

package body sb_pkg is
  type counter_t is protected body
    variable n : integer := 0;
    procedure inc(by : integer := 1) is begin n := n + by; end procedure;
    impure function get return integer is begin return n; end function;
    procedure reset is begin n := 0; end procedure;
  end protected body;

  type queue_t is protected body
    type node;
    type node_ptr is access node;
    type node is record
      v   : integer;
      nxt : node_ptr;
    end record;
    variable head, tail : node_ptr;
    variable pushes : counter_t;
    variable base : integer := 10;
    variable limit : integer := base * 2;
    procedure note is begin pushes.inc; end procedure;
    procedure push(v : integer) is
      variable p : node_ptr;
    begin
      p := new node'(v, null);
      if tail = null then head := p; else tail.nxt := p; end if;
      tail := p;
      note;
    end procedure;
    impure function pop return integer is
      variable p : node_ptr;
      variable v : integer;
    begin
      p := head;
      v := p.v;
      head := p.nxt;
      if head = null then tail := null; end if;
      deallocate(p);
      return v;
    end function;
    impure function empty return boolean is begin return head = null; end function;
    impure function size return natural is
      variable p : node_ptr := head;
      variable k : natural := 0;
    begin
      while p /= null loop
        k := k + 1;
        p := p.nxt;
      end loop;
      return k;
    end function;
    impure function describe return string is
    begin
      return "queue(" & integer'image(size) & " of " & integer'image(pushes.get)
        & ", limit " & integer'image(limit) & ")";
    end function;
    procedure push_all(a : integer_vector) is
    begin
      for i in a'range loop push(a(i)); end loop;
    end procedure;
  end protected body;
end package body;

package shared_pkg is
  shared variable events : work.sb_pkg.counter_t;
end package;

use work.shared_pkg.all;
entity producer is
  port (done : out boolean := false);
end entity;

architecture a of producer is
begin
  process
  begin
    events.inc(100);
    wait for 5 ns;
    done <= true;
    wait;
  end process;
end architecture;

use work.sb_pkg.all;
use work.shared_pkg.all;
entity protected_types is
end entity;

architecture a of protected_types is
  signal done : boolean;
  signal clk : bit := '0';
  procedure fill(variable q : inout queue_t; n : natural) is
  begin
    for i in 1 to n loop q.push(i * 3); end loop;
  end procedure;
begin
  u : entity work.producer port map (done => done);
  clk <= not clk after 1 ns when now < 8 ns;
  ticks : process (clk)
  begin
    if clk'event and clk = '1' then events.inc; end if;
  end process;
  main : process
    variable q, q2 : queue_t;
    variable local : counter_t;
    variable s : integer := 0;
  begin
    fill(q, 4);
    q2.push_all((7, 8, 9));
    local.inc(3);
    local.inc;
    report q.describe & " " & q2.describe & " local=" & integer'image(local.get);
    while not q.empty loop s := s + q.pop; end loop;
    report "sum=" & integer'image(s) & " empty=" & boolean'image(q.empty)
      & " q2.size=" & integer'image(q2.size);
    wait until done;
    wait for 10 ns;
    report "events=" & integer'image(events.get);
    events.reset;
    report "after reset=" & integer'image(events.get);
    wait;
  end process;
end architecture;
