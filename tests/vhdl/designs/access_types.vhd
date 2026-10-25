-- Access types (§5.4): an incomplete type for a linked list, allocators with
-- a qualified expression or a subtype, .all, implicit dereference of
-- elements and array indexing, 'length of a designated array, DEALLOCATE
-- and null.
entity access_types is end entity;
architecture a of access_types is
  type node;
  type node_ptr is access node;
  type node is record
    val  : integer;
    next_n : node_ptr;
  end record;
  type int_ptr is access integer;
  type str_ptr is access string;
  type iv is array (natural range <>) of integer;
  type iv_ptr is access iv;
begin
  process
    variable head, p, q : node_ptr;
    variable ip : int_ptr := new integer'(5);
    variable sp : str_ptr;
    variable vp : iv_ptr;
    variable sum : integer := 0;
  begin
    for i in 1 to 4 loop
      head := new node'(i * 10, head);
    end loop;
    p := head;
    while p /= null loop
      sum := sum + p.val;
      p := p.next_n;
    end loop;
    report "sum=" & integer'image(sum) & " head=" & integer'image(head.all.val);
    ip.all := ip.all + 1;
    report "ip=" & integer'image(ip.all);
    sp := new string'("hello");
    sp(1) := 'J';
    report "sp=" & sp.all & " len=" & integer'image(sp'length) & " " & integer'image(sp.all'high);
    vp := new iv(0 to 3);
    for i in vp'range loop vp(i) := i * i; end loop;
    report "vp(3)=" & integer'image(vp(3)) & " len=" & integer'image(vp'length);
    -- free the list
    p := head;
    while p /= null loop
      q := p.next_n;
      deallocate(p);
      p := q;
    end loop;
    head := null;
    deallocate(ip);
    report "freed " & boolean'image(ip = null) & " " & boolean'image(head = null);
    wait;
  end process;
end architecture;
