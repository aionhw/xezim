-- VHDL-2019 STD.REFLECTION: subtype mirrors ('REFLECT of a type) and value
-- mirrors ('REFLECT of an object) of enumeration, integer, floating,
-- physical, record, array and access types: names, literals, bounds,
-- elements, units, conversions between a mirror and its class's mirror.
use std.textio.all;
use std.reflection.all;
entity refl19 is end entity;
architecture a of refl19 is
  type color is (red, green, blue);
  type rec is record a : integer; b : color; end record;
  type arr is array (2 to 4) of integer;
  type dist is range 0 to 1000 units mm; m = 1000 mm; end units;
  subtype small is integer range 3 downto 1;
  type iptr is access integer;
begin
  process
    variable l : line;
    variable c : color := green;
    variable r : rec := (7, blue);
    variable v : arr := (10, 20, 30);
    variable d : dist := 5 mm;
    variable x : real := 2.5;
    variable vm : value_mirror;
    variable sm : subtype_mirror;
    variable p : iptr;
  begin
    sm := color'reflect;
    write(l, sm.simple_name & " " & type_class'image(sm.get_type_class) & " " & sm.to_enumeration.enumeration_literal(0).image
      & " " & index'image(sm.to_enumeration.length) & " " & sm.to_enumeration.right.image & " " & integer'image(sm.to_enumeration.enumeration_literal("blue").pos));
    writeline(output, l);
    vm := c'reflect;
    write(l, vm.to_enumeration.image & " " & integer'image(vm.to_enumeration.pos) & " " & vm.get_subtype_mirror.simple_name);
    writeline(output, l);
    sm := rec'reflect;
    write(l, sm.simple_name & " " & index'image(sm.to_record.length) & " " & sm.to_record.element_name(0) & " " & index'image(sm.to_record.element_index("b")) & " " & sm.to_record.element_subtype(1).simple_name);
    writeline(output, l);
    vm := r'reflect;
    write(l, integer'image(vm.to_record.get(0).to_integer.value) & " " & vm.to_record.get("b").to_enumeration.image);
    writeline(output, l);
    vm := v'reflect;
    write(l, integer'image(vm.to_array.get(3).to_integer.value) & " " & index'image(vm.to_array.get_subtype_mirror.left) & " " & index'image(vm.to_array.get_subtype_mirror.length) & " " & vm.to_array.get_subtype_mirror.element_subtype.simple_name & " " & index'image(vm.to_array.get_subtype_mirror.dimensions) & " " & vm.to_array.get_subtype_mirror.simple_name);
    writeline(output, l);
    vm := d'reflect;
    write(l, integer'image(vm.to_physical.value) & " " & index'image(vm.to_physical.unit_index) & " " & vm.to_physical.get_subtype_mirror.unit_name(1) & " " & integer'image(vm.to_physical.get_subtype_mirror.scale("m")) & " " & index'image(vm.to_physical.get_subtype_mirror.units_length));
    writeline(output, l);
    vm := x'reflect;
    write(l, vm.to_floating.image & " " & vm.get_subtype_mirror.simple_name);
    writeline(output, l);
    sm := small'reflect;
    write(l, sm.simple_name & " " & integer'image(sm.to_integer.left.value) & " " & integer'image(sm.to_integer.low.value) & " " & boolean'image(sm.to_integer.ascending) & " " & index'image(sm.to_integer.length));
    writeline(output, l);
    vm := p'reflect;
    write(l, boolean'image(vm.to_access.is_null) & " " & type_class'image(vm.get_value_class)
      & " " & vm.get_subtype_mirror.to_access.designated_subtype.simple_name);
    writeline(output, l);
    sm := real'reflect;
    wait;
  end process;
end architecture;
