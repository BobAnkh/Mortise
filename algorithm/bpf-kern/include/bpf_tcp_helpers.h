/* SPDX-License-Identifier: GPL-2.0 */
#ifndef __BPF_MORTISE_TCP_H
#define __BPF_MORTISE_TCP_H
#include "vmlinux.h"
// #include <stdbool.h>
// #include <stdio.h>
// #include <linux/types.h>
#include <linux/const.h>
// #include <linux/param.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_tracing.h>
#include "bpf_time_helpers.h"
#include "bpf_minmax_helpers.h"
#include "bpf_div.h"

#define BPF_STRUCT_OPS(name, args...) \
SEC("struct_ops/"#name) \
BPF_PROG(name, args)

/* TCP socket options */
#ifndef TCP_CONGESTION
#define TCP_CONGESTION          13      /* Congestion control algorithm */
#endif

#ifndef SOL_TCP
#define SOL_TCP 6
#endif
#ifndef LL_MAX_HEADER
#define LL_MAX_HEADER 128
#endif
#ifndef MAX_HEADER
#define MAX_HEADER (LL_MAX_HEADER + 48)
#endif
#ifndef TCP_CA_NAME_MAX
#define TCP_CA_NAME_MAX	16
#endif
/* TCP initial congestion window as per rfc6928 */
#ifndef TCP_INIT_CWND
#define TCP_INIT_CWND		10
#endif
#ifndef tcp_jiffies32
#define tcp_jiffies32 ((__u32)bpf_jiffies64())
#endif
#ifndef TCP_ECN_OK
#define	TCP_ECN_OK		1
#endif
#ifndef TCP_ECN_QUEUE_CWR
#define	TCP_ECN_QUEUE_CWR	2
#endif
#ifndef TCP_ECN_DEMAND_CWR
#define	TCP_ECN_DEMAND_CWR	4
#endif
#ifndef TCP_ECN_SEEN
#define	TCP_ECN_SEEN		8
#endif
#ifndef TCP_CONG_NEEDS_ECN
#define TCP_CONG_NEEDS_ECN	0x2
#endif
#ifndef TCP_INFINITE_SSTHRESH
#define TCP_INFINITE_SSTHRESH	0x7fffffff
#endif
/* Algorithm can be set on socket without CAP_NET_ADMIN privileges */
#ifndef TCP_CONG_NON_RESTRICTED
#define TCP_CONG_NON_RESTRICTED 0x1
#endif
#ifndef L1_CACHE_SHIFT
#define L1_CACHE_SHIFT		5
#endif
#ifndef L1_CACHE_BYTES
#define L1_CACHE_BYTES		(1 << L1_CACHE_SHIFT)
#endif
#ifndef L1_CACHE_ALIGN
#define L1_CACHE_ALIGN(x) __ALIGN_KERNEL(x, L1_CACHE_BYTES)
#endif
#ifndef MAX_TCP_HEADER
#define MAX_TCP_HEADER	L1_CACHE_ALIGN(128 + MAX_HEADER)
#endif
#ifndef GSO_MAX_SIZE
#define GSO_MAX_SIZE		65536
#endif

#ifndef container_of
#define container_of(ptr, type, member)				\
	({							\
		void *__mptr = (void *)(ptr);			\
		((type *)(__mptr - offsetof(type, member)));	\
	})
#endif

#ifndef LONG_MAX
#define LONG_MAX (~0UL>>1)
#endif
#ifndef MAX_JIFFY_OFFSET
#define MAX_JIFFY_OFFSET ((LONG_MAX >> 1)-1)
#endif

#ifndef likely
#define likely(x)	__builtin_expect(!!(x), 1)
#endif
#ifndef unlikely
#define unlikely(x)	__builtin_expect(!!(x), 0)
#endif

#define ___PASTE(a,b) a##b
#define __PASTE(a,b) ___PASTE(a,b)

/* Optimization barrier */
/* The "volatile" is due to gcc bugs */
#ifndef barrier
#define barrier() __asm__ __volatile__("": : :"memory")
#endif


static __always_inline bool before(__u32 seq1, __u32 seq2)
{
	return (__s32)(seq1-seq2) < 0;
}
#define after(seq2, seq1) 	before(seq1, seq2)

static __always_inline struct inet_connection_sock *inet_csk(const struct sock *sk)
{
	return (struct inet_connection_sock *)sk;
}

static __always_inline void *inet_csk_ca(const struct sock *sk)
{
	return (void *)inet_csk(sk)->icsk_ca_priv;
}

static __always_inline struct tcp_sock *tcp_sk(const struct sock *sk)
{
	return (struct tcp_sock *)sk;
}

static __always_inline bool tcp_in_slow_start(const struct tcp_sock *tp)
{
	return tp->snd_cwnd < tp->snd_ssthresh;
}

static __always_inline bool tcp_is_cwnd_limited(const struct sock *sk)
{
	const struct tcp_sock *tp = tcp_sk(sk);

	/* If in slow start, ensure cwnd grows to twice what was ACKed. */
	if (tcp_in_slow_start(tp))
		return tp->snd_cwnd < 2 * tp->max_packets_out;

	return !!BPF_CORE_READ_BITFIELD(tp, is_cwnd_limited);
}

static __always_inline bool tcp_cc_eq(const char *a, const char *b)
{
	int i;

	for (i = 0; i < TCP_CA_NAME_MAX; i++) {
		if (a[i] != b[i])
			return false;
		if (!a[i])
			break;
	}

	return true;
}

static inline unsigned int tcp_left_out(const struct tcp_sock *tp)
{
	return tp->sacked_out + tp->lost_out;
}

static inline unsigned int tcp_packets_in_flight(const struct tcp_sock *tp)
{
	return tp->packets_out - tcp_left_out(tp) + tp->retrans_out;
}

static inline __u32 tcp_stamp_us_delta(__u64 t1, __u64 t0)
{
	return max_t(__s64, t1 - t0, 0);
}

/* Minimum RTT in usec. ~0 means not available. */
static inline __u32 tcp_min_rtt(const struct tcp_sock *tp)
{
	return minmax_get(&tp->rtt_min);
}

static inline __u32 prandom_u32(void)
{
	return bpf_get_prandom_u32();
}

static inline __u32 prandom_u32_max(__u32 ep_ro)
{
	return (__u32)(((__u64) prandom_u32() * ep_ro) >> 32);
}

static inline __u32 tcp_slow_start(struct tcp_sock *tp, __u32 acked)
{
	u32 cwnd = min(tp->snd_cwnd_cnt + acked, tp->snd_ssthresh);

	acked -= cwnd - tp->snd_cwnd_cnt;
	tp->snd_cwnd_cnt = min(cwnd, tp->snd_cwnd_clamp);

	return acked;
}

static inline void tcp_cong_avoid_ai(struct tcp_sock *tp, __u32 w, __u32 acked)
{
		/* If credits accumulated at a higher w, apply them gently now. */
	if (tp->snd_cwnd_cnt >= w) {
		tp->snd_cwnd_cnt = 0;
		tp->snd_cwnd = tp->snd_cwnd + 1;
	}

	tp->snd_cwnd_cnt += acked;
	if (tp->snd_cwnd_cnt >= w) {
		u32 delta = tp->snd_cwnd_cnt / w;

		tp->snd_cwnd_cnt -= delta * w;
		tp->snd_cwnd = tp->snd_cwnd + delta;
	}
	tp->snd_cwnd = min(tp->snd_cwnd, tp->snd_cwnd_clamp);
}

/*
 * Following functions are taken from kernel sources and
 * break aliasing rules in their original form.
 *
 * While kernel is compiled with -fno-strict-aliasing,
 * perf uses -Wstrict-aliasing=3 which makes build fail
 * under gcc 4.4.
 *
 * Using extra __may_alias__ type to allow aliasing
 * in this case.
 */
typedef __u8  __attribute__((__may_alias__))  __u8_alias_t;
typedef __u16 __attribute__((__may_alias__)) __u16_alias_t;
typedef __u32 __attribute__((__may_alias__)) __u32_alias_t;
typedef __u64 __attribute__((__may_alias__)) __u64_alias_t;

static __always_inline void __read_once_size(const volatile void *p, void *res, int size)
{
	switch (size) {
	case 1: *(__u8_alias_t  *) res = *(volatile __u8_alias_t  *) p; break;
	case 2: *(__u16_alias_t *) res = *(volatile __u16_alias_t *) p; break;
	case 4: *(__u32_alias_t *) res = *(volatile __u32_alias_t *) p; break;
	case 8: *(__u64_alias_t *) res = *(volatile __u64_alias_t *) p; break;
	default:
		barrier();
		__builtin_memcpy((void *)res, (const void *)p, size);
		barrier();
	}
}

static __always_inline void __write_once_size(volatile void *p, void *res, int size)
{
	switch (size) {
	case 1: *(volatile  __u8_alias_t *) p = *(__u8_alias_t  *) res; break;
	case 2: *(volatile __u16_alias_t *) p = *(__u16_alias_t *) res; break;
	case 4: *(volatile __u32_alias_t *) p = *(__u32_alias_t *) res; break;
	case 8: *(volatile __u64_alias_t *) p = *(__u64_alias_t *) res; break;
	default:
		barrier();
		__builtin_memcpy((void *)p, (const void *)res, size);
		barrier();
	}
}

#ifndef READ_ONCE
#define READ_ONCE(x)					\
({							\
	union { typeof(x) __val; char __c[1]; } __u =	\
		{ .__c = { 0 } };			\
	__read_once_size(&(x), __u.__c, sizeof(x));	\
	__u.__val;					\
})
#endif

#ifndef WRITE_ONCE
#define WRITE_ONCE(x, val)				\
({							\
	union { typeof(x) __val; char __c[1]; } __u =	\
		{ .__val = (val) }; 			\
	__write_once_size(&(x), __u.__c, sizeof(x));	\
	__u.__val;					\
})
#endif


/**
 * abs - return absolute value of an argument
 * @x: the value.  If it is unsigned type, it is converted to signed type first.
 *     char is treated as if it was signed (regardless of whether it really is)
 *     but the macro's return type is preserved as char.
 *
 * Return: an absolute value of x.
 */
#define abs(x)	__abs_choose_expr(x, long long,				\
		__abs_choose_expr(x, long,				\
		__abs_choose_expr(x, int,				\
		__abs_choose_expr(x, short,				\
		__abs_choose_expr(x, char,				\
		__builtin_choose_expr(					\
			__builtin_types_compatible_p(typeof(x), char),	\
			(char)({ signed char __x = (x); __x<0?-__x:__x; }), \
			((void)0)))))))

#define __abs_choose_expr(x, type, other) __builtin_choose_expr(	\
	__builtin_types_compatible_p(typeof(x),   signed type) ||	\
	__builtin_types_compatible_p(typeof(x), unsigned type),		\
	({ signed type __x = (x); __x < 0 ? -__x : __x; }), other)

static inline unsigned long __generic_cmpxchg_local(volatile void *ptr,
		unsigned long old_val, unsigned long new_val, int size)
{
	// unsigned long flags, prev;
	unsigned long prev;

	/*
	 * Sanity checking, compile-time.
	 */
	// if (size == 8 && sizeof(unsigned long) != 8)
	// 	wrong_size_cmpxchg(ptr);

	// raw_local_irq_save(flags);
	switch (size) {
	case 1: prev = *(__u8 *)ptr;
		if (prev == old_val)
			*(__u8 *)ptr = (__u8)new_val;
		break;
	case 2: prev = *(__u16 *)ptr;
		if (prev == old_val)
			*(__u16 *)ptr = (__u16)new_val;
		break;
	case 4: prev = *(__u32 *)ptr;
		if (prev == old_val)
			*(__u32 *)ptr = (__u32)new_val;
		break;
	case 8: prev = *(__u64 *)ptr;
		if (prev == old_val)
			*(__u64 *)ptr = (__u64)new_val;
		break;
	// default:
	// 	wrong_size_cmpxchg(ptr);
	}
	// raw_local_irq_restore(flags);
	return prev;
}

#define generic_cmpxchg_local(ptr, o, n) ({					\
	((__typeof__(*(ptr)))__generic_cmpxchg_local((ptr), (unsigned long)(o),	\
			(unsigned long)(n), sizeof(*(ptr))));			\
})

#define cmpxchg generic_cmpxchg_local

#endif
