/* §35.5.4: exports declared inside a package that the module IMPORTS
   (wildcard in one testbench, by name in the other). Own names. */
extern void p_write(int addr, int data);
extern int  p_read(int addr);

int p_drive(void)
{
    p_write(2, 17);
    return p_read(2);
}
