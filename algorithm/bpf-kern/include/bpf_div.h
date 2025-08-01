#ifndef __BPF_DIV_TCP_H
#define __BPF_DIV_TCP_H
#include "vmlinux.h"
#include <bpf/bpf_helpers.h>

/**
 * do_div - returns 2 values: calculate remainder and update new dividend
 * @n: uint64_t dividend (will be updated)
 * @base: uint32_t divisor
 *
 * Summary:
 * ``uint32_t remainder = n % base;``
 * ``n = n / base;``
 *
 * Return: (uint32_t)remainder
 *
 * NOTE: macro parameter @n is evaluated multiple times,
 * beware of side effects!
 */
# define do_div(n,base) ({					\
	__u32 __base = (base);				\
	__u32 __rem;						\
	__rem = ((__u64)(n)) % __base;			\
	(n) = ((__u64)(n)) / __base;				\
	__rem;							\
})

static __always_inline __u64 div64_u64(__u64 dividend, __u64 divisor)
{
	return dividend / divisor;
}

static __always_inline __s64 div64_s64(__s64 dividend, __s64 divisor)
{
    bool aneg = dividend < 0;
    bool bneg = divisor < 0;
    // get the absolute positive value of both
    u64 adiv = aneg ? -dividend : dividend;
    u64 bdiv = bneg ? -divisor : divisor;
    // Do udiv
    u64 out = div64_u64(adiv, bdiv);
    // Make output negative if one or the other is negative, not both
    return aneg != bneg ? -out : out;

}

#define div64_long(x, y) div64_u64((x), (y))
#define div64_ul(x, y)   div64_u64((x), (y))
#define div_u64(x, y) div64_u64((x), (y))

#endif /* __BPF_DIV_TCP_H */
