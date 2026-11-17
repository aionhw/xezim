#include <stdio.h>
#include "svdpi.h"

int oa_inspect_write(
    const char *tag,
    svOpenArrayHandle h,
    int exp_dims,
    int exp_bytes,
    int exp_left,
    int exp_right,
    int exp_size,
    int exp_first,
    int exp_last
) {
    int dims = svDimensions(h);
    int bytes = svSizeOfArray(h);
    int left = svLeft(h, 1);
    int right = svRight(h, 1);
    int size = svSize(h, 1);
    int first = size > 0 ? *(int *)svGetArrElemPtr1(h, svLeft(h, 1)) : -1;
    int last = size > 0 ? *(int *)svGetArrElemPtr1(h, svRight(h, 1)) : -1;
    if (dims != exp_dims || bytes != exp_bytes || left != exp_left
        || right != exp_right || size != exp_size
        || first != exp_first || last != exp_last) {
        printf(
            "FAIL %s dims=%d(%d) bytes=%d(%d) left=%d(%d) right=%d(%d) size=%d(%d) first=%d(%d) last=%d(%d)\n",
            tag, dims, exp_dims, bytes, exp_bytes, left, exp_left,
            right, exp_right, size, exp_size, first, exp_first, last, exp_last
        );
        return 0;
    }
    if (size > 0) {
        for (int i = svLow(h, 1); i <= svHigh(h, 1); i++) {
            *(int *)svGetArrElemPtr1(h, i) = 100 + i;
        }
    }
    printf("PASS %s\n", tag);
    return 1;
}

int oa_inspect_write_inout(
    const char *tag,
    svOpenArrayHandle h,
    int exp_dims,
    int exp_bytes,
    int exp_left,
    int exp_right,
    int exp_size,
    int exp_first,
    int exp_last
) {
    return oa_inspect_write(
        tag, h, exp_dims, exp_bytes, exp_left, exp_right, exp_size, exp_first, exp_last
    );
}

int oa_check_in(
    const char *tag,
    const svOpenArrayHandle h,
    int exp_bytes,
    int exp_left,
    int exp_right,
    int exp_first,
    int exp_last
) {
    int bytes = svSizeOfArray(h);
    int left = svLeft(h, 1);
    int right = svRight(h, 1);
    int size = svSize(h, 1);
    int first = size > 0 ? *(int *)svGetArrElemPtr1(h, svLeft(h, 1)) : -1;
    int last = size > 0 ? *(int *)svGetArrElemPtr1(h, svRight(h, 1)) : -1;
    if (bytes != exp_bytes || left != exp_left || right != exp_right
        || first != exp_first || last != exp_last) {
        printf(
            "FAIL %s bytes=%d(%d) left=%d(%d) right=%d(%d) first=%d(%d) last=%d(%d)\n",
            tag, bytes, exp_bytes, left, exp_left, right, exp_right,
            first, exp_first, last, exp_last
        );
        return 0;
    }
    printf("PASS %s\n", tag);
    return 1;
}

int oa_2d_inout(const char *tag, svOpenArrayHandle h) {
    int dims = svDimensions(h);
    int bytes = svSizeOfArray(h);
    int l1 = svLeft(h, 1), r1 = svRight(h, 1);
    int l2 = svLeft(h, 2), r2 = svRight(h, 2);
    if (dims != 2 || bytes != 24 || l1 != 0 || r1 != 1 || l2 != 0 || r2 != 2) {
        printf(
            "FAIL %s dims=%d bytes=%d d1=%d:%d d2=%d:%d\n",
            tag, dims, bytes, l1, r1, l2, r2
        );
        return 0;
    }
    for (int i = svLow(h, 1); i <= svHigh(h, 1); i++) {
        for (int j = svLow(h, 2); j <= svHigh(h, 2); j++) {
            int *p = (int *)svGetArrElemPtr2(h, i, j);
            if (!p || *p != (i + 1) * 10 + j) {
                printf("FAIL %s [%d][%d]=%d\n", tag, i, j, p ? *p : -999);
                return 0;
            }
            *p = 200 + i * 10 + j;
        }
    }
    printf("PASS %s\n", tag);
    return 1;
}
