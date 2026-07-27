pub const MATH_C: &str = r#"
/* ===== Stdlib.Math ===== */
#include <math.h>
#include <stdlib.h>
#include <time.h>

static double certo_math_pi(void)  { return M_PI; }
static double certo_math_e(void)   { return M_E;  }
static double certo_math_sin(double x)  { return sin(x);   }
static double certo_math_cos(double x)  { return cos(x);   }
static double certo_math_tan(double x)  { return tan(x);   }
static double certo_math_asin(double x) { return asin(x);  }
static double certo_math_acos(double x) { return acos(x);  }
static double certo_math_atan(double x) { return atan(x);  }
static double certo_math_atan2(double y, double x) { return atan2(y, x); }
static double certo_math_log(double x)  { return log(x);   }
static double certo_math_log2(double x) { return log2(x);  }
static double certo_math_log10(double x){ return log10(x); }
static double certo_math_exp(double x)  { return exp(x);   }
static double certo_math_pow(double x, double y) { return pow(x, y); }
static double certo_math_hypot(double a, double b) { return hypot(a, b); }
static double certo_math_clamp(double x, double lo, double hi) {
    if (x < lo) return lo;
    if (x > hi) return hi;
    return x;
}
static int64_t certo_math_clamp_int(int64_t x, int64_t lo, int64_t hi) {
    if (x < lo) return lo;
    if (x > hi) return hi;
    return x;
}
static double certo_math_sign(double x) {
    if (x > 0.0) return 1.0;
    if (x < 0.0) return -1.0;
    return 0.0;
}
static int64_t certo_math_sign_int(int64_t x) {
    if (x > 0) return 1;
    if (x < 0) return -1;
    return 0;
}
static double certo_math_trunc(double x) { return trunc(x); }
static int _certo_math_rng_seeded = 0;
static double certo_math_random(void) {
    if (!_certo_math_rng_seeded) {
        srand((unsigned int)time(NULL));
        _certo_math_rng_seeded = 1;
    }
    return (double)rand() / ((double)RAND_MAX + 1.0);
}
"#;
