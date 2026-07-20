/* C consumer: links the import library (add.lib) and calls into add.dll. */
#include <stdio.h>
#include "add.h"   /* declares: int64_t certo_add(int64_t, int64_t); */

int main(void) {
    long long a = 2, b = 3;
    long long result = certo_add(a, b);
    printf("certo_add(%lld, %lld) = %lld\n", a, b, result);
    return 0;
}
