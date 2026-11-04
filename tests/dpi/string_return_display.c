/* Imports declared `string` in SV; see string_return_display_test.sv. */
const char *srd_msg(int i) { return i ? "second message" : "hello from C"; }
