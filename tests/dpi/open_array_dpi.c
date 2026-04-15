int sum4(const int *a) {
    if (!a) return 0;
    return a[0] + a[1] + a[2] + a[3];
}

void fill4(int *a) {
    if (!a) return;
    a[0] = 10;
    a[1] = 20;
    a[2] = 30;
    a[3] = 40;
}

void bump4(int *a) {
    if (!a) return;
    for (int i = 0; i < 4; i++) a[i] += 1;
}
