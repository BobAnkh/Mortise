#ifndef __BPF_DEF_HELPERS_H
#define __BPF_DEF_HELPERS_H
#include <stdbool.h>
#include <stdio.h>
#include <linux/types.h>
#include <linux/tcp.h>
#include <linux/const.h>
#include <linux/param.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_core_read.h>
#include <bpf/bpf_tracing.h>

#ifndef TCP_CA_NAME_MAX
#define TCP_CA_NAME_MAX	16
#endif
/* A single data point for our parameterized min-max tracker */
struct minmax_sample {
	__u32	t;	/* time measurement was taken */
	__u32	v;	/* value measured */
};

/* State for the parameterized min-max tracker */
struct minmax {
	struct minmax_sample s[3];
};

struct sock_common {
	unsigned char	skc_state;
	__u16		skc_num;
} __attribute__((preserve_access_index));

enum sk_pacing {
	SK_PACING_NONE		= 0,
	SK_PACING_NEEDED	= 1,
	SK_PACING_FQ		= 2,
};

struct sock {
	struct sock_common	__sk_common;
#define sk_state		__sk_common.skc_state
	unsigned long		sk_pacing_rate;
	unsigned long		sk_max_pacing_rate;
	__u32			sk_pacing_status; /* see enum sk_pacing */
	__u8			sk_pacing_shift;
} __attribute__((preserve_access_index));

struct inet_sock {
	struct sock		sk;
} __attribute__((preserve_access_index));

struct inet_connection_sock {
	struct inet_sock	  icsk_inet;
	__u8			  icsk_ca_state:6,
				  icsk_ca_setsockopt:1,
				  icsk_ca_dst_locked:1;
	struct {
		__u8		  pending;
	} icsk_ack;
	__u64			  icsk_ca_priv[104 / sizeof(__u64)];
} __attribute__((preserve_access_index));

struct request_sock {
	struct sock_common		__req_common;
} __attribute__((preserve_access_index));

struct tcp_sock {
	struct inet_connection_sock	inet_conn;

	__u32	rcv_nxt;
	__u32	snd_nxt;
	__u32	snd_una;
	__u32	mss_cache;	/* Cached effective mss, not including SACKS */
	__u32	window_clamp;
	__u8	ecn_flags;
	__u32	delivered;
	__u32	delivered_ce;
	__u32	snd_cwnd;
	__u32	snd_cwnd_cnt;
	__u32	snd_cwnd_clamp;
	__u32	lost;		/* Total data packets lost incl. rexmits */
	__u32	app_limited;	/* limited until "delivered" reaches this val */
	__u64	delivered_mstamp; /* time we reached "delivered" */
	__u32	lost_out;	/* Lost packets			*/
	__u32	sacked_out;	/* SACK'd packets			*/
	__u32	snd_ssthresh;
	__u8	syn_data:1,	/* SYN includes data */
		syn_fastopen:1,	/* SYN includes Fast Open option */
		syn_fastopen_exp:1,/* SYN includes Fast Open exp. option */
		syn_fastopen_ch:1, /* Active TFO re-enabling probe */
		syn_data_acked:1,/* data in SYN is acked by SYN-ACK */
		save_syn:1,	/* Save headers of SYN packet */
		is_cwnd_limited:1,/* forward progress limited by snd_cwnd? */
		syn_smc:1;	/* SYN includes SMC */
	__u32	max_packets_out;
	__u32	lsndtime;
	__u32	prior_cwnd;
	__u64	tcp_wstamp_ns;	/* departure time for next sent data packet */
	__u64	tcp_clock_cache; /* cache last tcp_clock_ns() (see tcp_mstamp_refresh()) */
	__u64	tcp_mstamp;	/* most recent packet received/sent */
	__u32	srtt_us;	/* smoothed round trip time << 3 in usecs */
	struct  minmax rtt_min;
	__u32	packets_out;	/* Packets which are "in flight"	*/
	__u32	retrans_out;	/* Retransmitted packets out		*/
	bool	is_mptcp;
} __attribute__((preserve_access_index));

enum inet_csk_ack_state_t {
	ICSK_ACK_SCHED	= 1,
	ICSK_ACK_TIMER  = 2,
	ICSK_ACK_PUSHED = 4,
	ICSK_ACK_PUSHED2 = 8,
	ICSK_ACK_NOW = 16	/* Send the next ACK immediately (once) */
};

enum tcp_ca_event {
	CA_EVENT_TX_START = 0,
	CA_EVENT_CWND_RESTART = 1,
	CA_EVENT_COMPLETE_CWR = 2,
	CA_EVENT_LOSS = 3,
	CA_EVENT_ECN_NO_CE = 4,
	CA_EVENT_ECN_IS_CE = 5,
};

struct ack_sample {
	__u32 pkts_acked;
	__s32 rtt_us;
	__u32 in_flight;
} __attribute__((preserve_access_index));

struct rate_sample {
	__u64  prior_mstamp; /* starting timestamp for interval */
	__u32  prior_delivered;	/* tp->delivered at "prior_mstamp" */
	__s32  delivered;		/* number of packets delivered over interval */
	long interval_us;	/* time for tp->delivered to incr "delivered" */
	__u32 snd_interval_us;	/* snd interval for delivered packets */
	__u32 rcv_interval_us;	/* rcv interval for delivered packets */
	long rtt_us;		/* RTT of last (S)ACKed packet (or -1) */
	int  losses;		/* number of packets marked lost upon ACK */
	__u32  acked_sacked;	/* number of packets newly (S)ACKed upon ACK */
	__u32  prior_in_flight;	/* in flight before this ACK */
	bool is_app_limited;	/* is sample from packet with bubble in pipe? */
	bool is_retrans;	/* is sample from retransmission? */
	bool is_ack_delayed;	/* is this (likely) a delayed ACK? */
} __attribute__((preserve_access_index));

struct tcp_congestion_ops {
	char name[TCP_CA_NAME_MAX];
	__u32 flags;

	/* initialize private data (optional) */
	void (*init)(struct sock *sk);
	/* cleanup private data  (optional) */
	void (*release)(struct sock *sk);

	/* return slow start threshold (required) */
	__u32 (*ssthresh)(struct sock *sk);
	/* do new cwnd calculation (required) */
	void (*cong_avoid)(struct sock *sk, __u32 ack, __u32 acked);
	/* call before changing ca_state (optional) */
	void (*set_state)(struct sock *sk, __u8 new_state);
	/* call when cwnd event occurs (optional) */
	void (*cwnd_event)(struct sock *sk, enum tcp_ca_event ev);
	/* call when ack arrives (optional) */
	void (*in_ack_event)(struct sock *sk, __u32 flags);
	/* new value of cwnd after loss (required) */
	__u32  (*undo_cwnd)(struct sock *sk);
	/* hook for packet ack accounting (optional) */
	void (*pkts_acked)(struct sock *sk, const struct ack_sample *sample);
	/* override sysctl_tcp_min_tso_segs */
	__u32 (*min_tso_segs)(struct sock *sk);
	/* returns the multiplier used in tcp_sndbuf_expand (optional) */
	__u32 (*sndbuf_expand)(struct sock *sk);
	/* call when packets are delivered to update cwnd and pacing rate,
	 * after all the ca_state processing. (optional)
	 */
	void (*cong_control)(struct sock *sk, const struct rate_sample *rs);
	/* get info for inet_diag (optional) */
	size_t (*get_info)(struct sock *sk, __u32 ext, int *attr,
			   union tcp_cc_info *info);
	void *owner;
};

#endif