/*
 *   Copyright (c) 2026 
 *   All rights reserved.
 */
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>

typedef struct { int64_t value; int8_t scale; } certo_decimal_t;
typedef enum {
    CERTO_ROUND_HALF_UP, CERTO_ROUND_HALF_DOWN, CERTO_ROUND_HALF_EVEN,
    CERTO_ROUND_UP, CERTO_ROUND_DOWN, CERTO_ROUND_CEILING, CERTO_ROUND_FLOOR,
    CERTO_ROUND_TO_INCREMENT,
} certo_rounding_tag_t;
typedef struct { certo_rounding_tag_t tag; certo_decimal_t step; } certo_rounding_mode_t;

static int64_t certo_round_apply(int64_t truncated, int64_t remainder, int64_t divisor, certo_rounding_tag_t mode) {
    bool negative = remainder < 0;
    int64_t abs_remainder = negative ? -remainder : remainder;
    bool exactly_half = (2 * abs_remainder == divisor);
    bool over_half     = (2 * abs_remainder > divisor);
    switch (mode) {
        case CERTO_ROUND_UP: return abs_remainder == 0 ? truncated : (negative ? truncated - 1 : truncated + 1);
        case CERTO_ROUND_DOWN: return truncated;
        case CERTO_ROUND_CEILING: return (abs_remainder == 0 || negative) ? truncated : truncated + 1;
        case CERTO_ROUND_FLOOR: return (abs_remainder == 0 || !negative) ? truncated : truncated - 1;
        case CERTO_ROUND_HALF_UP:
            if (abs_remainder == 0) return truncated;
            if (exactly_half || over_half) return negative ? truncated - 1 : truncated + 1;
            return truncated;
        case CERTO_ROUND_HALF_DOWN:
            if (abs_remainder == 0) return truncated;
            if (over_half) return negative ? truncated - 1 : truncated + 1;
            return truncated;
        case CERTO_ROUND_HALF_EVEN: {
            if (abs_remainder == 0) return truncated;
            if (over_half) return negative ? truncated - 1 : truncated + 1;
            if (exactly_half) {
                bool truncated_is_odd = (truncated % 2 != 0);
                if (truncated_is_odd) return negative ? truncated - 1 : truncated + 1;
                return truncated;
            }
            return truncated;
        }
        default: fprintf(stderr, "PANIC\n"); return 0;
    }
}

void decimal_align(certo_decimal_t* a, certo_decimal_t* b) {
    while (a->scale < b->scale) { a->value *= 10; a->scale++; }
    while (b->scale < a->scale) { b->value *= 10; b->scale++; }
}

certo_decimal_t certo_decimal_round_mode(certo_decimal_t d, int8_t places, certo_rounding_mode_t mode) {
    if (mode.tag == CERTO_ROUND_TO_INCREMENT) { fprintf(stderr,"use roundToIncrement\n"); certo_decimal_t z={0,0}; return z; }
    if (d.scale <= places) return d;
    int8_t excess = d.scale - places;
    int64_t divisor = 1;
    for (int i = 0; i < excess; i++) divisor *= 10;
    int64_t truncated = d.value / divisor;
    int64_t remainder = d.value % divisor;
    int64_t rounded = certo_round_apply(truncated, remainder, divisor, mode.tag);
    certo_decimal_t r = { .value = rounded, .scale = places };
    return r;
}

certo_decimal_t certo_decimal_round_to_increment(certo_decimal_t d, certo_decimal_t step, certo_rounding_mode_t mode) {
    certo_decimal_t a = d, b = step;
    decimal_align(&a, &b);
    if (b.value == 0) { fprintf(stderr, "PANIC: step must be nonzero\n"); certo_decimal_t z={0,0}; return z; }
    int64_t steps_truncated = a.value / b.value;
    int64_t remainder       = a.value % b.value;
    certo_rounding_tag_t inner_mode = (mode.tag == CERTO_ROUND_TO_INCREMENT) ? CERTO_ROUND_HALF_UP : mode.tag;
    int64_t steps = certo_round_apply(steps_truncated, remainder, b.value, inner_mode);
    certo_decimal_t r = { .value = steps * b.value, .scale = a.scale };
    return r;
}

certo_decimal_t certo_decimal_div_round(certo_decimal_t a, certo_decimal_t b, int8_t places, certo_rounding_mode_t mode) {
    if (b.value == 0) { fprintf(stderr,"PANIC: division by zero\n"); certo_decimal_t z={0,0}; return z; }
    int8_t guard = 6;
    int64_t guard_factor = 1;
    for (int i = 0; i < guard; i++) guard_factor *= 10;
    int exponent = (int)places + (int)b.scale - (int)a.scale + guard;
    bool exponent_negative = exponent < 0;
    int abs_exponent = exponent_negative ? -exponent : exponent;
    __int128 scale_pow = 1;
    for (int i = 0; i < abs_exponent; i++) scale_pow *= 10;
    __int128 numerator = exponent_negative ? (__int128)a.value / scale_pow : (__int128)a.value * scale_pow;
    __int128 guarded_quotient = numerator / b.value;
    int64_t truncated_with_guard = (int64_t)(guarded_quotient / guard_factor);
    int64_t remainder_for_round  = (int64_t)(guarded_quotient % guard_factor);
    int64_t rounded = certo_round_apply(truncated_with_guard, remainder_for_round, guard_factor, mode.tag);
    certo_decimal_t r = { .value = rounded, .scale = places };
    return r;
}

#define MK(v,s) ((certo_decimal_t){.value=(v),.scale=(s)})
#define MODE(t) ((certo_rounding_mode_t){.tag=(t)})
int total=0, passed=0;
void check(const char* label, certo_decimal_t got, int64_t ev, int8_t es) {
    total++;
    bool pass = got.value == ev && got.scale == es;
    if (pass) passed++;
    printf("%-55s %s\n", label, pass ? "PASS" : "FAIL ***");
}

int main(void) {
    check("HalfUp 1.5->0dp=2",     certo_decimal_round_mode(MK(15,1),0,MODE(CERTO_ROUND_HALF_UP)), 2,0);
    check("HalfUp -1.5->0dp=-2",   certo_decimal_round_mode(MK(-15,1),0,MODE(CERTO_ROUND_HALF_UP)), -2,0);
    check("HalfDown 1.5->0dp=1",   certo_decimal_round_mode(MK(15,1),0,MODE(CERTO_ROUND_HALF_DOWN)), 1,0);
    check("HalfEven 2.5->0dp=2",   certo_decimal_round_mode(MK(25,1),0,MODE(CERTO_ROUND_HALF_EVEN)), 2,0);
    check("HalfEven 3.5->0dp=4",   certo_decimal_round_mode(MK(35,1),0,MODE(CERTO_ROUND_HALF_EVEN)), 4,0);
    check("Up 1.1->0dp=2",         certo_decimal_round_mode(MK(11,1),0,MODE(CERTO_ROUND_UP)), 2,0);
    check("Down -1.9->0dp=-1",     certo_decimal_round_mode(MK(-19,1),0,MODE(CERTO_ROUND_DOWN)), -1,0);
    check("Ceiling -1.9->0dp=-1",  certo_decimal_round_mode(MK(-19,1),0,MODE(CERTO_ROUND_CEILING)), -1,0);
    check("Floor -1.1->0dp=-2",    certo_decimal_round_mode(MK(-11,1),0,MODE(CERTO_ROUND_FLOOR)), -2,0);
    check("divRound 100.00/3,2dp=33.33", certo_decimal_div_round(MK(10000,2),MK(3,0),2,MODE(CERTO_ROUND_HALF_UP)), 3333,2);
    check("divRound -10/3,2dp=-3.33",    certo_decimal_div_round(MK(-10,0),MK(3,0),2,MODE(CERTO_ROUND_HALF_UP)), -333,2);
    check("divRound 1.00/8,2dp HalfEven=0.12", certo_decimal_div_round(MK(100,2),MK(8,0),2,MODE(CERTO_ROUND_HALF_EVEN)), 12,2);
    check("incr 19.97->0.05 HalfUp=19.95", certo_decimal_round_to_increment(MK(1997,2),MK(5,2),MODE(CERTO_ROUND_HALF_UP)), 1995,2);
    check("incr 19.975->0.05 tie HalfDown=19.95", certo_decimal_round_to_increment(MK(19975,3),MK(5,2),MODE(CERTO_ROUND_HALF_DOWN)), 19950,3);
    check("incr -19.97->0.05 HalfUp=-19.95", certo_decimal_round_to_increment(MK(-1997,2),MK(5,2),MODE(CERTO_ROUND_HALF_UP)), -1995,2);
    printf("\n%d/%d passed\n", passed, total);
    return passed==total ? 0 : 1;
}