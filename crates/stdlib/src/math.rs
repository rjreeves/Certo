pub const MATH_C: &str = r#"
/* ===== Stdlib.Math ===== */
#include <math.h>

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
"#;

pub const MATH_CERTO: &str = r#"
// Stdlib.Math — mathematical functions

// Constants
extern fn Math.pi(): Float
extern fn Math.e(): Float

// Trigonometry (angles in radians)
extern fn Math.sin(x: Float): Float
extern fn Math.cos(x: Float): Float
extern fn Math.tan(x: Float): Float
extern fn Math.asin(x: Float): Float
extern fn Math.acos(x: Float): Float
extern fn Math.atan(x: Float): Float
extern fn Math.atan2(y: Float, x: Float): Float

// Logarithms and exponential
extern fn Math.log(x: Float): Float
extern fn Math.log2(x: Float): Float
extern fn Math.log10(x: Float): Float
extern fn Math.exp(x: Float): Float

// Geometry
extern fn Math.hypot(a: Float, b: Float): Float

// Clamping
extern fn Math.clamp(x: Float, lo: Float, hi: Float): Float
extern fn Math.clampInt(x: Int, lo: Int, hi: Int): Int
"#;
