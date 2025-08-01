/* SPDX-License-Identifier: GPL-2.0 */
#ifndef _BPF_MORTISE_TIME64_H
#define _BPF_MORTISE_TIME64_H

#include "vmlinux.h"
#include <bpf/bpf_helpers.h>
#include "bpf_div.h"
extern unsigned long CONFIG_HZ __kconfig;
#define HZ CONFIG_HZ
// TODO: need further check
// #define HZ 250

#define MSEC_PER_SEC	1000L
#define USEC_PER_MSEC	1000L
#define NSEC_PER_USEC	1000L
#define NSEC_PER_MSEC	1000000L
#define USEC_PER_SEC	1000000L
#define NSEC_PER_SEC	1000000000L
#define FSEC_PER_SEC	1000000000000000LL
#define USEC_PER_JIFFY	(USEC_PER_SEC / HZ)

#ifndef LONG_MAX
#define LONG_MAX (~0UL>>1)
#endif
#ifndef MAX_JIFFY_OFFSET
#define MAX_JIFFY_OFFSET ((LONG_MAX >> 1)-1)
#endif

static inline unsigned long _msecs_to_jiffies(const unsigned int m)
{
	if (HZ <= MSEC_PER_SEC && !(MSEC_PER_SEC % HZ))
		return (m + (MSEC_PER_SEC / HZ) - 1) / (MSEC_PER_SEC / HZ);
	else
	{
		if (m > (MAX_JIFFY_OFFSET + (HZ / MSEC_PER_SEC) - 1)/(HZ / MSEC_PER_SEC))
			return MAX_JIFFY_OFFSET;
		return m * (HZ / MSEC_PER_SEC);
	}
}

static inline u64 tcp_clock_ns(void)
{
	return bpf_ktime_get_ns();
}

static inline u64 tcp_clock_us(void)
{
	return div_u64(tcp_clock_ns(), NSEC_PER_USEC);
}

static __always_inline unsigned long msecs_to_jiffies(const unsigned int m)
{
	if ((int)m < 0)
		return MAX_JIFFY_OFFSET;
	return _msecs_to_jiffies(m);
}

#endif /* _BPF_MORTISE_TIME64_H */
