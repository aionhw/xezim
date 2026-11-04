/* IEEE 1800-2023 §26.3 / §35.5.4: only imported declarations reach C. */
int scope_callback(int value) { return value + 10; }
int scope_action(int *value) { *value = 11; return 0; }
