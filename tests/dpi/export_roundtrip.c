/* §35.5.4 DPI export round-trip: an imported context function calls back
   into exported SystemVerilog functions/tasks. Own names, not from any
   external source. */
extern int sv_scale(int x);       /* exported SV function: x * 3      */
extern int sv_combine(int a, int b);
extern void sv_record(int v);     /* exported SV task: stores v       */

int c_roundtrip(int x)
{
    int s = sv_scale(x);          /* -> 3x     */
    int c = sv_combine(s, x);     /* -> 3x + x */
    sv_record(c);                 /* task write-back */
    return c + 1;
}
