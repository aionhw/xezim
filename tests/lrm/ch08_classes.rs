//! IEEE 1800-2023 clause 8: classes.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use crate::harness::{Order, check};

// §8.14: Overridden members (property shadowing via base handle)
#[test]
fn c8_14_shadow_base_handle() {
    check(
        "8.14_shadow_base_handle",
        include_str!("sv/8.14_shadow_base_handle.sv"),
        "t8_14",
        Order::Exact,
        &["T|a|d.v=2 b.v=1"],
        &[],
    );
}

// §8.15: Super (super.new args, super.method, chained handles)
#[test]
fn c8_15_chain_handle_write() {
    check(
        "8.15_chain_handle_write",
        include_str!("sv/8.15_chain_handle_write.sv"),
        "t8_15",
        Order::Exact,
        &[
            "T|a|n.val=1 n.next.val=2 n.next.next.val=3",
            "T|b|n.next.next==n 0",
        ],
        &[],
    );
}

// §8.25: Parameterized classes: fixed array property sized by a value parameter
#[test]
fn c8_25_param_sized_array() {
    check(
        "8.25_param_sized_array",
        include_str!("sv/8.25_param_sized_array.sv"),
        "t8_25a",
        Order::Exact,
        &["T|a|items='{0, 0, 0} size=3", "T|b|pop=20"],
        &[],
    );
}

// §8.25.1: parameterized classes
#[test]
fn c8_25_parameterized_classes() {
    // Known gap (§8.25): `8.25`: 2 reference lines differ from xezim's 2, e.g. reference `ext W=5 depth=10 bits=5`, xezim `ext W=5 depth=X bits=1`
    check(
        "c8_param",
        include_str!("sv/c8_param.sv"),
        "c8_param",
        Order::Exact,
        &[
            "T|8.25|default 20 4 bits=32",
            "T|8.25|string bc depth=2",
            "T|8.25|typedef spec depth=8 bits=8",
            "T|8.25.1|static per spec int4=3 str2=1 byte8=1",
            "T|8.25.1|cnt c1=4 c5=6 c0=1",
            "T|8.25|type param maxv byte=-1 bit4=1111",
            "T|8.25|no default 6",
            "T|8.25|real q 1.500000",
        ],
        &["T|8.25|ext W=5 depth=", "T|8.25|typedef in param "],
    );
}

// §8.25/§6.20.2 (issue #423): class-scope array constants — header value
// parameter with unpacked dims, class localparam array, static array
// property initializer — read per specialization.
#[test]
fn c8_25_class_param_array_reads() {
    check(
        "8.25_class_param_array_reads",
        include_str!("sv/8.25_class_param_array_reads.sv"),
        "t8_25pa",
        Order::Exact,
        &[
            "T|8.25|spec LP 4 7",
            "T|8.25|spec fn 5 6",
            "T|8.25|default 7 4",
            "T|8.25|static arr 20 30 3",
        ],
        &[],
    );
}

// §8.25: Parameterized classes: static method called through a handle
#[test]
fn c8_25_static_via_handle() {
    check(
        "8.25_static_via_handle",
        include_str!("sv/8.25_static_via_handle.sv"),
        "t8_25b",
        Order::Exact,
        &["T|a|handle.static=2 handle.nonstatic=2 scoped=2"],
        &[],
    );
}

// §8.26.6: Interface classes: $cast between interface class handles
#[test]
fn c8_26_cast_between_iface_classes() {
    // Known gap (§8.26.6): `a` reference `cast iface->iface=1`, xezim `cast iface->iface=0`
    // Known gap (§8.26.6): `b`: xezim misses `get=6`
    check(
        "8.26_cast_between_iface_classes",
        include_str!("sv/8.26_cast_between_iface_classes.sv"),
        "t8_26",
        Order::Exact,
        &["T|c|cast iface->class=1"],
        &["T|a|", "T|b|"],
    );
}

// §8.26.5, §8.26.7: interface classes: type access, partial implementation
#[test]
fn c8_26_interface_classes() {
    // Known gap (§8.26.9): a forward `typedef interface class` does not parse; this variant of the
    // probe drops that line (the reference lines are those of the full probe).
    check(
        "c8_iface4",
        include_str!("sv/c8_iface4.sv"),
        "c8_iface4",
        Order::Exact,
        &[
            "T|8.26.7|partial f=11/11 g=22",
            "T|8.26.5|type access 5",
            "T|8.26|null iface handle 1",
        ],
        &[],
    );
}

// §8.5: Object properties and object parameter data (byte dyn array element)
#[test]
fn c8_5_byte_dyn_p() {
    check(
        "8.5_byte_dyn_p",
        include_str!("sv/8.5_byte_dyn_p.sv"),
        "t8_5",
        Order::Exact,
        &["T|a|cls dyn='{0, -1} fa='{0, -2} mod='{-1} el=-1"],
        &[],
    );
}

// §8.5: Locator method results of a class queue property compared (ar.a.min() == ar.a.max())
#[test]
fn c8_5_class_queue_min_max_compare_panic() {
    check(
        "8.5_class_queue_min_max_compare_panic",
        include_str!("sv/8.5_class_queue_min_max_compare_panic.sv"),
        "t7_12",
        Order::Exact,
        &["T|b|cmp=0"],
        &[],
    );
}

// §8.5: Two-level handle member reads (g.h.n, g.h.tag, g.get().show()) when the design has an unpacked struct variable
#[test]
fn c8_5_struct_var_breaks_nested_string() {
    check(
        "8.5_struct_var_breaks_nested_string",
        include_str!("sv/8.5_struct_var_breaks_nested_string.sv"),
        "t8_5c",
        Order::Exact,
        &[
            "T|a|[show] [abc] [show] [abc] [show]",
            "T|b|assigned [abc] len=3 eq=1 n=7",
        ],
        &[],
    );
}

// §8.5: %s of a string property read through two handle levels
#[test]
fn c8_5_two_level_string_prop_display() {
    check(
        "8.5_two_level_string_prop_display",
        include_str!("sv/8.5_two_level_string_prop_display.sv"),
        "t17",
        Order::Exact,
        &["T|a|two-level [s] k=4", "T|b|concat [s!] eq=1"],
        &[],
    );
}

// §8.4, §8.7, §8.10, §8.11, §8.12, §8.13, ...: objects, constructors, static members, this/super, inheritance, casting
#[test]
fn c8_classes() {
    // Known gap (§8.17): `8.17` reference `extends args a=5`, xezim `extends args a=0`
    // Known gap (§8.8): `8.8` reference `typed ctor Derived`, xezim `typed ctor Base`
    check(
        "c8_basic",
        include_str!("sv/c8_basic.sv"),
        "c8_basic",
        Order::Exact,
        &[
            "T|8.7|defaults id=7 name=dflt count=1",
            "T|8.4|uninit data=00 lg=xxxx name='dflt' r=0.000000 arr0=0 qsz=0",
            "T|8.7|args id=3 name=two count=2 seq=2",
            "T|8.10|static via handle 2 2",
            "T|8.9|static shared 5",
            "T|8.11|this 1",
            "T|8.4|null 1 1",
            r#"T|8.5|props q='{1} aa='{"k":9} dyn='{0, 0, -1}"#,
            "T|8.13|a=7 who=Derived nv=Derived.nv b.nv=Base.nv calc=63",
            "T|8.14|shadow d.v=2 b.v=1",
            "T|8.20|implicit virtual D2 a=3",
            "T|8.16|downcast ok D2",
            "T|8.16|bad downcast fail as expected",
            "T|8.16|cast null 1",
            "T|8.15|chain 3 2",
            "T|8.12|shallow o1.y=1 o1.in.x=7 o2.y=9 same=1",
            "T|8.12|copy no ctor count=2 id=7 q='{1}",
            "T|8.12|copy deep q p='{1} p2='{1, 4}",
            "T|8.24|null after 1",
        ],
        &["T|8.17|", "T|8.8|"],
    );
}

// §8.4, §8.20: nested handles, method chaining, polymorphism
#[test]
fn c8_classes_deep() {
    check(
        "c8_deep",
        include_str!("sv/c8_deep.sv"),
        "c8_deep",
        Order::Exact,
        &[
            "T|8.4|queue of handles 0 1 2 size=3",
            "T|8.20|poly in queue Sub:Item(40) Item(0)",
            "T|8.4|assoc of handles 9 Sub:Item(40) exists=0",
            "T|8.4|array of handles 22",
            "T|8.4|dyn of handles null0=1 v1=5",
            "T|8.12|clone independent 1 99",
            "T|8.15|chained call 2 Sub:Item(40)",
            "T|8.20|virtual call from base ctor -> 1",
            "T|8.25|class-type param Sub:Item(10) tag=s",
            "T|8.10|singleton 5 same=1",
            "T|8.6|default args 157 127 359",
            "T|8.4|handle in struct 8",
            "T|8.23|class enum BUSY 1",
            "T|7|queue delete handles 2 3",
            "T|8.16|cast to int via enum 1",
        ],
        &[],
    );
}

// §8.6, §8.9, §8.18, §8.19: methods, out-of-block declarations, abstract classes
#[test]
fn c8_classes_misc() {
    check(
        "c8_misc",
        include_str!("sv/c8_misc.sv"),
        "c8_misc",
        Order::Exact,
        &[
            "T|8.18|peek local 5 gp=13 pub=7",
            "T|8.19|const ci=3 inst=1,2,4 sc=11",
            "T|8.9|ids 0 1 2 n=3",
            "T|8.20|leaf hi",
            "T|8.20|fin hi",
            "T|8.20|base hi",
            "T|8.10|task in class v=3 t=6",
            "T|8.6|recursion 120",
            "T|8.9|static init b=8",
            "T|8.5|event prop hits=1",
        ],
        &[],
    );
}

// §8.21, §8.23, §8.24, §8.26, §8.27: virtual methods, abstract classes, nested classes, interface classes
#[test]
fn c8_virtual_methods() {
    // Known gap (§8.26.6): `8.26.6`: xezim misses `cast between ifaces 6`
    check(
        "c8_virt",
        include_str!("sv/c8_virt.sv"),
        "c8_virt",
        Order::Exact,
        &[
            "T|8.21|abstract sq:4.0",
            "T|8.21|abstract2 shape:3.0",
            "T|8.26|iface get=5 cnt=1",
            "T|8.26.3|extends ifaces -1 1",
            "T|8.26.4|iface param 8 4",
            "T|8.24|extern f=12 sf=99",
            "T|8.24|extern task o=11 t=1",
            "T|8.27|typedef class 42",
            "T|8.23|nested 11 1",
            "T|8.23|nested scope 8",
            "T|8.23|scope res 1",
        ],
        &["T|8.26.6|"],
    );
}
