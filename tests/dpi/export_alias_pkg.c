/* §35.5.4: a C body calls back into exported SV subroutines through their
   C LINKAGE names: an alias (`export "DPI-C" c_store = task sv_store;`),
   exports declared in a package no module imports, and an alias at
   compilation-unit scope. The alias used to be dropped (the trampoline
   emitted the SV name) and package-scope exports were never registered, so
   the first call died with `undefined symbol`. Own names. */
extern void c_store(int addr, int data);      /* module alias -> sv_store   */
extern void pkg_store(int addr, int data);    /* unimported package task    */
extern int  pkg_load(int addr);               /* unimported package function*/
extern void unit_tag(int v);                  /* $unit alias -> sv_tag      */

int c_drive(void)
{
    c_store(3, 0x2A);          /* 42 into the module memory        */
    pkg_store(3, 0x2A);        /* 42 into the package's own memory */
    unit_tag(7);               /* tag = 7 at $unit                 */
    return pkg_load(3);        /* read back through the package    */
}
