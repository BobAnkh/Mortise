#include "bpf_minmax_helpers.h"
#include "bpf_tcp_helpers.h"
#include "bpf_limits.h"
#include "bpf_time_helpers.h"
#include "vmlinux.h"
#include "mortise_app.h"

// __attribute__((no_builtin("memcpy")))

char _license[] SEC("license") = "GPL";

/* Scale factor for rate in pkt/uSec unit to avoid truncation in bandwidth
 * estimation. The rate unit ~= (1500 bytes / 1 usec / 2^24) ~= 715 bps.
 * This handles bandwidths from 0.06pps (715bps) to 256Mpps (3Tbps) in a u32.
 * Since the minimum window is >=4 packets, the lower bound isn't
 * an issue. The upper bound isn't an issue with existing technologies.
 */
#define MAX_CHUNK_LEN 50
#define BW_SCALE 24
#define BW_UNIT (1 << BW_SCALE)
#define COPA_SCALE 8
#define COPA_UNIT (1 << COPA_SCALE)

#define MAX_MIM_LIMIT 64

#define MIM_HASH 0
#define MAX_ARRAY_SIZE 100000
// the BINARY_SEARCH_LIMIT is the result of log_2 (MAX_ARRAY_SIZE) (~=17)
#define BINARY_SEARCH_LIMIT 20
#define INTERVALS_PER_CYCLE 4
#define ROUNDS_PER_INTERVAL 1
// when trade-off change, wait 1 interval for transition between 2 cycles
#define CYCLE_TRANSITION_INTERVALS 1
// when probing over 10 cycles, stay still
#define MAX_PROBING_INTERVALS 50
#define TRADE_OFF_DEFAULT_MOVING_STEP 70
// still move, but with a smaller step
#define TRADE_OFF_VAGUE_MOVING_STEP 20
// may use multiply instead of add, and moving step should grow when moving in same direction
#define INIT_PROBING_EPS 0
#define MAX_PROBING_EPS 70
#define EWMA_ALPHA 900
#define EWMV_EWMA_ALPHA 600
#define EWMA_WND_LENGTH 10

// if current step is too small to stat the trade-off line, enlarge it by 20
#define PROBING_EPS_STEP 20
#define MIN_RATE_DIFF_RATIO_FOR_GRAD 20
#define MIN_RTT_DIFF_RATIO_FOR_GRAD 20
#define MIN_GRAD_DIFF_RATIO 200
#define MAX_BASE_PARAM 500
#define MIN_BASE_PARAM 100
#define MAX_PROBING_DELTA 600
#define MIN_PROBING_DELTA 5

// trade-off gap under the same parameter before and after > 10%
#define NETWORK_UNSTABLE_DIFF_RATIO 100
#define QOE_MIN_DIFF_RATIO 80
// 3%
#define ABNORMAL_DIFF_RATIO 20

#define USE_VAR
// #define USE_QOE
// #define FAST_REACT

#define EWMA_SCALE 10000
#define VAR_SCALE 10000

const u64 EWMA_WEIGHT[10] = {
	4000, 2400, 1440, 864, 518, 311, 187, 112, 67, 40
};

const char DIRECTION_STRING[3][8] = { "Delay", "Tput", "Hold" };

// #define SEARCH
// #define LOGGING
// #define PKT_LEVEL_LOG
// #define INT_LEVEL_LOG
// #define RTT_LEVEL_LOG
// #define TRADE_OFF
#define MEASUREMENT
#define REPORT
// #define TEST

struct rtt_entry {
	u64 rtt;
	u64 time;
};

struct app_sk_stg sk_stg_map SEC(".maps");

struct report_data_elem {
	u32 rtt;
	u32 acked_bytes;
	u32 lost_bytes;
	u32 timestamp;
};

struct report_entry {
	u32 flow_id;
	s16 chunk_id;
	u16 chunk_len;
	struct report_data_elem data_array[MAX_CHUNK_LEN];
};

struct {
	__uint(type, BPF_MAP_TYPE_RINGBUF);
	__uint(max_entries, 256 * 1024 * 1024 /* 256 MB */);
} rb SEC(".maps");

struct array_map {
	__uint(type, BPF_MAP_TYPE_HASH);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, u64);
	__uint(max_entries, MAX_ARRAY_SIZE);
};

struct rtt_array {
	__uint(type, BPF_MAP_TYPE_HASH);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct rtt_entry);
	__uint(max_entries, MAX_ARRAY_SIZE);
};

struct outer_map {
	__uint(type, BPF_MAP_TYPE_HASH_OF_MAPS);
	__uint(max_entries, MAX_MIM_LIMIT);
	__type(key, u32);
	__array(values, struct array_map);
};

struct outer_map2 {
	__uint(type, BPF_MAP_TYPE_HASH_OF_MAPS);
	__uint(max_entries, MAX_MIM_LIMIT);
	__type(key, u32);
	__array(values, struct rtt_array);
};

struct array_map copa_increase SEC(".maps");
struct rtt_array copa_rtt SEC(".maps");

struct outer_map2 mim_rtt SEC(".maps") = {
	.values = { [0] = (void *)&copa_rtt },
};

struct outer_map mim_increase SEC(".maps") = {
	.values = { [0] = (void *)&copa_increase },
};

enum copa_direction {
	None,
	Up, // cwnd is increasing
	Down, // cwnd is decreasing
};

enum copa_mode {
	Default,
	TCPCoop,
	Loss,
};

struct copa_ringbuf {
	u32 head;
	u32 tail;
	u32 len;
};

struct callback_ctx {
	struct copa_ringbuf *ringbuf;
	u64 earliest;
	u64 max_rtt;
	u64 min_rtt;
};

struct copa_velocity_state {
	u64 velocity;
	enum copa_direction direction;
	// number of rtts direction has remained same
	u64 num_times_direction_same;
	// updated every srtt
	u64 last_recorded_cwnd_bytes;
	u64 last_cwnd_record_time;
	u64 time_since_direction;
};

struct probing_cycle_record {
	u64 bounce_intervals;
	// us ^ 2
	s64 base_param;
	u64 intervals_cnt;
	// probing trade-off changes by probing_eps up and down of current param
	s64 probing_eps;
	bool tcp_coop;
};

struct copa_info {
	struct minmax_u64 min_rtt;
	struct minmax_u64 standing_rtt;
	struct copa_ringbuf rtt_ringbuf;
	struct copa_ringbuf increase_ringbuf;
	struct copa_velocity_state velocity_state;
	struct probing_cycle_record trade_off_stg;
	struct report_entry entry;
	u64 last_report_timestamp;
	u64 first_timestamp;
};

struct {
	__uint(type, BPF_MAP_TYPE_SK_STORAGE);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct copa_info);
} copa_info_stg SEC(".maps");

struct {
	__uint(type, BPF_MAP_TYPE_SK_STORAGE);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, int);
} flow_id_stg SEC(".maps");

struct app_sk_stg rate_sk_stg SEC(".maps");

static const u32 min_rtt_window = 10 * USEC_PER_SEC; // 10 seconds
static const u32 standing_rtt_window = 100 * USEC_PER_MSEC; // 100 ms
static const u64 quantization_base = 1000;
static const u64 report_rtt_interval = quantization_base / 2;
static const u32 min_cwnd_segment = 4;

struct copa {
	// u64 bitrate;
	// time in microseconds
	// struct minmax_u64 min_rtt;
	// struct minmax_u64 standing_rtt;
	bool use_standing_rtt;
	bool is_slow_start;
	bool have_flow_id;
	u32 flow_id;
	/*
	 * delta_param determines how latency sensitive the algorithm is.
	 * Lower means it will maximize throughput at expense of delay.
	 * Higher value means it will minimize delay at expense of throughput.
	 * Default value is 500 / 1000.
	 */
	u64 delta_param;
	u64 default_param;
	// time at which cwnd was last doubled during slow start
	u64 last_cwnd_double_time;
	u32 ack_bytes_round;
	// data structure: u64_entry
	// struct copa_ringbuf rtt_ringbuf;
	// struct copa_ringbuf increase_ringbuf;
	// Maximum time till which to maintain history. It is minimum of 10s and 20 RTTs.
	u64 max_time;
	// Number of increases and decreases in the current `increase` window
	u32 num_increase;
	enum copa_mode mode;
	bool mode_switch;
	u32 total_acked_bytes;
	u32 cur_num_acked;
	u32 cur_num_losses;
	u32 report_acked_bytes;
	u32 report_lost_bytes;
	// End of the last window of tracking losses
	u64 prev_loss_cycle;
	// Loss rate in the previous cycle
	u64 prev_loss_rate;
	u64 last_report_time;
	// struct copa_velocity_state velocity_state;
};

static inline void velocity_reset(struct copa_velocity_state *state)
{
	state->velocity = 1;
	state->direction = None;
	state->num_times_direction_same = 0;
	state->last_recorded_cwnd_bytes = 0;
	state->last_cwnd_record_time = 0;
	state->time_since_direction = 0;
}

static inline void reset_trade_off_record(struct probing_cycle_record *record)
{
	record->base_param = quantization_base / 2;
	record->probing_eps = INIT_PROBING_EPS;
	record->tcp_coop = false;
}

static long find_minmax_rtt(struct bpf_map *map, u32 *key,
			    struct rtt_entry *val, struct callback_ctx *data)
{
	struct copa_ringbuf *ringbuf = data->ringbuf;
	if (ringbuf->head < ringbuf->tail) {
		if (*key >= ringbuf->head && *key < ringbuf->tail) {
			if (val->time > data->earliest) {
				data->max_rtt = max(data->max_rtt, val->rtt);
				data->min_rtt = min(data->min_rtt, val->rtt);
			}
		} else {
			bpf_map_delete_elem(map, key);
		}
	} else {
		if (*key >= ringbuf->head || *key < ringbuf->tail) {
			if (val->time > data->earliest) {
				data->max_rtt = max(data->max_rtt, val->rtt);
				data->min_rtt = min(data->min_rtt, val->rtt);
			}
		} else {
			bpf_map_delete_elem(map, key);
		}
	}
	return 0;
}

static inline void copa_init_ringbuf(struct copa_ringbuf *ringbuf)
{
	ringbuf->head = 0;
	ringbuf->tail = 0;
	ringbuf->len = 0;
}

/* Return rate in bytes per second, optionally with a gain.
 * The order here is chosen carefully to avoid overflow of u64. This should
 * work for input rates of up to 2.9Tbit/sec and gain of 2.89x.
 */
static inline u64 copa_rate_bytes_per_sec(struct sock *sk, u64 rate, int gain)
{
	unsigned int mss = tcp_sk(sk)->mss_cache;

	rate *= mss;
	rate *= gain;
	rate >>= COPA_SCALE;
	rate *= USEC_PER_SEC;
	return rate >> BW_SCALE;
}

/* Convert a Copa bw and gain factor to a pacing rate in bytes per second. */
static inline unsigned long copa_bw_to_pacing_rate(struct sock *sk, u32 bw,
						   int gain)
{
	u64 rate = bw;

	rate = copa_rate_bytes_per_sec(sk, rate, gain);
	rate = min_t(u64, rate, sk->sk_max_pacing_rate);
	return rate;
}

// cwnd_bytes = snd_cwnd*mss

static inline void copa_init_pacing_rate_from_rtt(struct sock *sk, int gain)
{
	struct tcp_sock *tp = tcp_sk(sk);
	u32 rtt_us;
	unsigned long rate;
	if (tp->srtt_us) { /* any RTT sample yet? */
		rtt_us = max(tp->srtt_us >> 3, 1U);
		u64 bw = (u64)tp->snd_cwnd * BW_UNIT;
		do_div(bw, rtt_us);
		rate = copa_bw_to_pacing_rate(sk, bw, gain);
	} else { /* no RTT sample yet */
		// 1000000 bps * gain(2)
		// in bytes per second
		rate = 125000 * gain;
		rate >>= COPA_SCALE;
	}

	sk->sk_pacing_rate = rate;
}

/* static inline u64 copa_get_delivery_rate(struct sock *sk, */
/* 					 const struct rate_sample *rs) */
/* { */
/* 	u64 bw = div_u64((u64)rs->delivered * BW_UNIT, (u64)rs->interval_us); */
/* 	return 8 * copa_bw_to_pacing_rate(sk, (u32)bw, 1 << COPA_SCALE); */
/* } */

static inline void copa_update_rate_stg(struct sock *sk,
					const struct rate_sample *rs)
{
	struct tcp_sock *tp = tcp_sk(sk);
	if (tp->srtt_us) { /* any RTT sample yet? */
		struct app_info *rate_stg =
			bpf_sk_storage_get(&rate_sk_stg, (void *)sk, NULL,
					   BPF_LOCAL_STORAGE_GET_F_CREATE);
		if (rate_stg) {
			rate_stg->req = rate_stg->resp;
			rate_stg->resp = sk->sk_pacing_rate / 2;
		}
	}
}

// now is in microseconds
static inline void clear_old_hist(struct sock *sk, u64 now, void *rtt_map,
				  void *increase_map, struct copa_info *stg)
{
	struct copa *copa = inet_csk_ca(sk);
	struct copa_ringbuf *rtt_ringbuf = &stg->rtt_ringbuf;
	struct copa_ringbuf *increase_ringbuf = &stg->increase_ringbuf;
	// BPF verifier has a weird limit on the for loop
	// so instead of deleting elements one by one here we binary search for the new head fast
	if ((now > copa->max_time) && (rtt_ringbuf->len > 1)) {
		u32 head = (int)rtt_ringbuf->head;
		u32 tail = (int)rtt_ringbuf->tail;
		u64 target = now - copa->max_time;
		// DONE: use binary search to find the first time that is larger than target to be the new had
		for (int i = 0; i < BINARY_SEARCH_LIMIT; i++) {
			u32 mid = (((tail + MAX_ARRAY_SIZE - head) %
				    MAX_ARRAY_SIZE) /
					   2 +
				   head) %
				  MAX_ARRAY_SIZE;
			struct rtt_entry *mid_val =
				bpf_map_lookup_elem(rtt_map, &(int){ mid });
			if (mid_val) {
				if (mid_val->time >= target) {
					tail = mid;
				} else {
					head = mid + 1;
				}
			}
		}
		u32 moving = (head + MAX_ARRAY_SIZE - rtt_ringbuf->head) %
			     MAX_ARRAY_SIZE;
		moving = min(moving, rtt_ringbuf->len - 1);
		rtt_ringbuf->head =
			(rtt_ringbuf->head + moving) % MAX_ARRAY_SIZE;
		rtt_ringbuf->len -= moving;
	}
	if (increase_ringbuf->len > 40) {
		u32 need_move = increase_ringbuf->len - 40;
		increase_ringbuf->head =
			(increase_ringbuf->head + need_move) % MAX_ARRAY_SIZE;
		increase_ringbuf->len = 40;
		copa->num_increase -= need_move;
	}
}

// rtt, rtt_min, srtt and now are in microseconds
static inline void new_rtt_sample(struct sock *sk, u64 rtt, u64 rtt_min,
				  u64 srtt, u64 now, struct copa_info *stg)
{
	struct copa *copa = inet_csk_ca(sk);
	// seems no need to update every time
	// copa->max_time = (u64)(10 * USEC_PER_SEC);

	// insert rtt sample, push back data
	struct copa_ringbuf *rtt_ringbuf = &stg->rtt_ringbuf;
	struct copa_ringbuf *increase_ringbuf = &stg->increase_ringbuf;
	void *rtt_map;
	void *increase_map;
	rtt_map = bpf_map_lookup_elem(&mim_rtt, &(u32){ copa->flow_id });
	increase_map =
		bpf_map_lookup_elem(&mim_increase, &(u32){ copa->flow_id });
	if (rtt_map && increase_map) {
		// update return 0 means success
		struct rtt_entry e = {
			.rtt = rtt,
			.time = now,
		};
		int n = bpf_map_update_elem(
			rtt_map, &(int){ rtt_ringbuf->tail }, &e, BPF_ANY);
		if (!n) {
			rtt_ringbuf->tail =
				(rtt_ringbuf->tail + 1) % MAX_ARRAY_SIZE;
			if (rtt_ringbuf->len < MAX_ARRAY_SIZE) {
				rtt_ringbuf->len++;
			} else {
				rtt_ringbuf->head = (rtt_ringbuf->head + 1) %
						    MAX_ARRAY_SIZE;
			}
		} else {
			bpf_printk("Failed to update rtt map");
		}
	} else {
		// This situation should never happen
		bpf_printk("Empty map found in new rtt sample");
		return;
	}

	// update increase (delete increase judgement, only to count)
	if (increase_ringbuf->len == 0) {
		int n = bpf_map_update_elem(increase_map,
					    &(int){ increase_ringbuf->tail },
					    &now, BPF_ANY);
		if (!n) {
			copa->num_increase += 1;
			increase_ringbuf->tail =
				(increase_ringbuf->tail + 1) % MAX_ARRAY_SIZE;
			if (increase_ringbuf->len < MAX_ARRAY_SIZE) {
				increase_ringbuf->len++;
			} else {
				increase_ringbuf->head =
					(increase_ringbuf->head + 1) %
					MAX_ARRAY_SIZE;
			}
		} else {
			bpf_printk("Failed to update increase map");
		}
	} else {
		int back = (increase_ringbuf->tail + MAX_ARRAY_SIZE - 1) %
			   MAX_ARRAY_SIZE;
		u64 *back_val = bpf_map_lookup_elem(increase_map, &back);
		if (back_val && *back_val < (now - 2 * rtt_min)) {
			int n = bpf_map_update_elem(
				increase_map, &(int){ increase_ringbuf->tail },
				&now, BPF_ANY);
			if (!n) {
				copa->num_increase += 1;
				increase_ringbuf->tail =
					(increase_ringbuf->tail + 1) %
					MAX_ARRAY_SIZE;
				if (increase_ringbuf->len < MAX_ARRAY_SIZE) {
					increase_ringbuf->len++;
				} else {
					increase_ringbuf->head =
						(increase_ringbuf->head + 1) %
						MAX_ARRAY_SIZE;
				}
			} else {
				bpf_printk("Failed to update increase map");
			}
		}
	}

	// clear old history
	clear_old_hist(sk, now, rtt_map, increase_map, stg);
}

static inline bool tcp_detected(struct sock *sk, u64 rtt_min, u64 srtt, u64 now,
				struct copa_info *stg)
{
	struct copa *copa = inet_csk_ca(sk);
	void *rtt_map = bpf_map_lookup_elem(&mim_rtt, &(u32){ copa->flow_id });
	struct copa_ringbuf *ringbuf = &stg->rtt_ringbuf;
	// Disable tcp-detect
	u64 min_rtt = U64_MAX;
	u64 max_rtt = 0;
	if (rtt_map && ringbuf->len > 0) {
		int back =
			(ringbuf->tail + MAX_ARRAY_SIZE - 1) % MAX_ARRAY_SIZE;
		struct rtt_entry *back_val =
			bpf_map_lookup_elem(rtt_map, &back);
		u64 earliest;
		if (back_val) {
			earliest = back_val->time - 10 * srtt;
		} else {
			earliest = now - 10 * srtt;
			bpf_printk("Back value is empty");
		}
		struct callback_ctx data = {
			.ringbuf = ringbuf,
			.earliest = earliest,
			.min_rtt = U64_MAX,
			.max_rtt = 0,
		};
		bpf_for_each_map_elem(rtt_map, find_minmax_rtt, &data, 0);
		min_rtt = data.min_rtt;
		max_rtt = data.max_rtt;
	}
	u64 thresh = rtt_min + (max_rtt - rtt_min) / 2 + 100;
	bool res;
	if (min_rtt > thresh) {
		res = true;
	} else {
		res = false;
	}
	return res;
}

// ATTENTION: keep in mind that delta_param is multiplied by 1000(quantization_base)
static inline void report_measurement(struct sock *sk, u64 rtt_min, u64 srtt,
				      u64 now, u32 acked, u32 lost,
				      struct copa_info *stg)
{
	struct probing_cycle_record *record = &stg->trade_off_stg;
	struct copa *copa = inet_csk_ca(sk);
	copa->cur_num_acked += acked;
	copa->cur_num_losses += lost;

#ifndef TRADE_OFF
	struct app_info *info =
		bpf_sk_storage_get(&sk_stg_map, (void *)sk, NULL, 0);
	if (info && !info->resp) {
		copa->default_param = info->req;
		record->base_param = info->req;
		record->bounce_intervals = 0;
		if (info->req <= 100) {
			// record->probing_eps = 100;
			record->bounce_intervals = 12;
		}
		// if (info->req < 8) {
		// 	record->tcp_coop = true;
		// 	copa->default_param = 35;
		// }
		record->probing_eps = 0;
		info->resp = 1;
	}
#endif

	if (now > copa->prev_loss_cycle + 2 * rtt_min) {
		if (copa->cur_num_losses + copa->cur_num_acked > 0) {
			copa->prev_loss_rate = (u64)copa->cur_num_losses *
					       quantization_base /
					       (u64)(copa->cur_num_losses +
						     copa->cur_num_acked);
		}
		copa->cur_num_losses = 0;
		copa->cur_num_acked = 0;
		copa->prev_loss_cycle = now;
	}

	if (copa->prev_loss_rate >= quantization_base / 30) {
		copa->mode = Loss;
	} else if (tcp_detected(sk, rtt_min, srtt, now, stg) ||
		   record->tcp_coop) {
		copa->mode = TCPCoop;
	} else {
		copa->mode = Default;
		copa->delta_param = copa->default_param;
	}

	if (copa->mode == Default || !copa->mode_switch) {
		copa->delta_param = copa->default_param;
	} else if (copa->mode == TCPCoop) {
		if (lost > 0) {
			copa->delta_param = 2 * copa->delta_param;
		} else {
			// delta = 1 / (1 + 1 / delta)
			copa->delta_param =
				copa->delta_param * quantization_base /
				(copa->delta_param + quantization_base);
		}
		if (copa->delta_param > copa->default_param) {
			copa->delta_param = copa->default_param;
		}
	} else if (copa->mode == Loss) {
		if (lost > 0) {
			copa->delta_param = 2 * copa->delta_param;
		}
		if (copa->delta_param > copa->default_param) {
			copa->delta_param = copa->default_param;
		}
	}
}

static inline void change_direction(u64 now,
				    struct copa_velocity_state *velocity_state,
				    enum copa_direction direction,
				    u32 cwnd_bytes)
{
	if (direction == velocity_state->direction) {
		return;
	}
	velocity_state->direction = direction;
	velocity_state->velocity = 1;
	velocity_state->time_since_direction = now;
	velocity_state->last_recorded_cwnd_bytes = cwnd_bytes;
}

static inline void
check_and_update_direction(struct sock *sk, u64 now, u64 srtt,
			   struct copa_velocity_state *velocity_state,
			   u32 cwnd_bytes, u32 acked_bytes)
{
	struct copa *copa = inet_csk_ca(sk);
	if (velocity_state->last_cwnd_record_time == 0) {
		velocity_state->last_cwnd_record_time = now;
		velocity_state->last_recorded_cwnd_bytes = cwnd_bytes;
		return;
	}
	copa->total_acked_bytes += acked_bytes;
	if (copa->total_acked_bytes >= cwnd_bytes) {
		enum copa_direction direction =
			(cwnd_bytes >
			 velocity_state->last_recorded_cwnd_bytes) ?
				Up :
				Down;
		if ((direction == velocity_state->direction) &&
		    (now - velocity_state->time_since_direction > 3 * srtt)) {
			velocity_state->velocity = 2 * velocity_state->velocity;
		} else if (direction != velocity_state->direction) {
			velocity_state->velocity = 1;
			velocity_state->time_since_direction = now;
		}
		velocity_state->direction = direction;
		velocity_state->last_cwnd_record_time = now;
		velocity_state->last_recorded_cwnd_bytes = cwnd_bytes;
		copa->total_acked_bytes = 0;
	}
}

SEC("struct_ops/bpf_copa_main")
void BPF_PROG(bpf_copa_main, struct sock *sk, const struct rate_sample *rs)
{
	struct tcp_sock *tp = tcp_sk(sk);
	struct copa *copa = inet_csk_ca(sk);
	long rtt = rs->rtt_us;
	u32 srtt = tcp_sk(sk)->srtt_us >> 3;
	u32 cwnd_bytes = tp->snd_cwnd * tp->mss_cache;
	u64 now = tcp_clock_us();
	enum copa_direction old_direction;
	copa->report_acked_bytes += rs->acked_sacked * tp->mss_cache;
	copa->report_lost_bytes += rs->losses * tp->mss_cache;
	if (!copa->have_flow_id) {
		u32 *flow_id =
			bpf_sk_storage_get(&flow_id_stg, (void *)sk, NULL, 0);
		if (flow_id) {
			copa->flow_id = *flow_id;
			copa->have_flow_id = true;
			bpf_printk("flow_id: %d", *flow_id);
		}
	}
	struct copa_info *stg =
		bpf_sk_storage_get(&copa_info_stg, (void *)sk, NULL,
				   BPF_LOCAL_STORAGE_GET_F_CREATE);
	if (stg) {
#ifdef REPORT
		struct report_entry *entry = &stg->entry;
		if (entry->chunk_len < MAX_CHUNK_LEN) {
			struct report_data_elem *elem =
				&entry->data_array[entry->chunk_len];
			elem->rtt = (u32)rtt;
			elem->acked_bytes = rs->acked_sacked * tp->mss_cache;
			u32 losses = rs->losses * tp->mss_cache;
			elem->lost_bytes = losses;
			elem->timestamp = (u32)(now - stg->first_timestamp);
			// bpf_printk("Record one new sample, chunk len: %lld", entry->chunk_len);
		}
		entry->chunk_len += 1;
		if (entry->chunk_len >= MAX_CHUNK_LEN ||
		    now - stg->last_report_timestamp > 200 * USEC_PER_MSEC) {
			entry->flow_id = copa->flow_id;
			struct report_entry *e = bpf_ringbuf_reserve(
				&rb, sizeof(struct report_entry), 0);
			if (e) {
				*e = *entry;
				bpf_ringbuf_submit(e, 0);
				stg->last_report_timestamp = now;
			}
			entry->chunk_len = 0;
			entry->chunk_id += 1;
			// bpf_printk("submit %d", entry->chunk_id);
		}
#endif
		if (rtt >= 0) {
			minmax_running_min_u64(&stg->min_rtt, min_rtt_window,
					       now, (u64)rtt);
			minmax_running_min_u64(&stg->standing_rtt,
					       (u64)srtt / 2, now, (u64)rtt);
		}
		u64 min_rtt = minmax_get_u64(&stg->min_rtt);
		new_rtt_sample(sk, rtt, min_rtt, (u64)srtt, now, stg);
	}

	if (copa->is_slow_start) {
		u32 new_cwnd = cwnd_bytes + rs->acked_sacked * tp->mss_cache;
		tp->snd_cwnd =
			min(new_cwnd / tp->mss_cache, tp->snd_cwnd_clamp);
	}

	if (!(now > 0 && copa->last_report_time + (u64)srtt *
							  report_rtt_interval /
							  quantization_base <
				 now)) {
		return;
	}
	if (stg) {
		u64 min_rtt = minmax_get_u64(&stg->min_rtt);
		u64 min_standing_rtt = minmax_get_u64(&stg->standing_rtt);
		if (min_standing_rtt < min_rtt) {
			return;
		}
		report_measurement(sk, min_rtt, srtt, now,
				   copa->report_acked_bytes,
				   copa->report_lost_bytes, stg);

		u64 delay_us;
		u32 acked_packets =
			(copa->report_acked_bytes + tp->mss_cache - 1) /
			tp->mss_cache;
		if (copa->use_standing_rtt) {
			delay_us = min_standing_rtt - min_rtt;
		} else {
			delay_us = rtt - min_rtt;
		}

		bool increase_cwnd;
		struct copa_velocity_state *velocity_state =
			&stg->velocity_state;
		struct probing_cycle_record *record = &stg->trade_off_stg;
		old_direction = velocity_state->direction;
		if (delay_us == 0) {
			increase_cwnd = true;
		} else {
			// in bytes per second
			u64 target_rate = tp->mss_cache * USEC_PER_SEC *
					  quantization_base /
					  (delay_us * copa->delta_param);
			if (record->intervals_cnt %
					    (record->bounce_intervals + 1) !=
				    0 &&
			    !copa->is_slow_start) {
				target_rate =
					target_rate * 1700 / quantization_base;
			}
			// bpf_printk("%lld bounce, intervals_cnt: %d", tcp_clock_us(), record->intervals_cnt);
			u64 current_rate =
				cwnd_bytes * USEC_PER_SEC / min_standing_rtt;
			increase_cwnd = target_rate >= current_rate;
		}

		if (!(increase_cwnd && copa->is_slow_start)) {
			check_and_update_direction(sk, now, srtt,
						   velocity_state, cwnd_bytes,
						   copa->report_acked_bytes);
		}

		u32 change = (acked_packets * tp->mss_cache * tp->mss_cache *
			      velocity_state->velocity * quantization_base) /
			     (copa->delta_param * cwnd_bytes);
		change = min(change, cwnd_bytes);
		if (increase_cwnd) {
			if (!copa->is_slow_start) {
				if (velocity_state->direction != Up &&
				    velocity_state->velocity > 1) {
					change_direction(now, velocity_state,
							 Up, cwnd_bytes);
				}
				u32 new_cwnd = cwnd_bytes + change;
				tp->snd_cwnd = min(new_cwnd / tp->mss_cache,
						   tp->snd_cwnd_clamp);
			}
		} else {
			if (velocity_state->direction != Down &&
			    velocity_state->velocity > 1) {
				change_direction(now, velocity_state, Down,
						 cwnd_bytes);
			}
			u32 new_cwnd = cwnd_bytes - change;
			new_cwnd =
				max(new_cwnd, min_cwnd_segment * tp->mss_cache);
			if (copa->is_slow_start) {
				new_cwnd = min(new_cwnd, cwnd_bytes >> 1);
			}
			tp->snd_cwnd = min(new_cwnd / tp->mss_cache,
					   tp->snd_cwnd_clamp);
			copa->is_slow_start = false;
		}

		if (old_direction == Down && velocity_state->direction == Up) {
			record->intervals_cnt += 1;
#ifdef REPORT
			struct report_entry *entry = &stg->entry;
			if (entry->chunk_len) {
				entry->flow_id = copa->flow_id;
				// the end of interval
				entry->chunk_id = -entry->chunk_id;
				struct report_entry *e = bpf_ringbuf_reserve(
					&rb, sizeof(struct report_entry), 0);
				if (e) {
					*e = *entry;
					bpf_ringbuf_submit(e, 0);
					stg->last_report_timestamp = now;
				}
			}
			entry->chunk_len = 0;
			entry->chunk_id = 1;
			// bpf_printk("submit %d", entry->chunk_id);
#endif
		}

		copa_init_pacing_rate_from_rtt(sk, 2 << COPA_SCALE);
		copa_update_rate_stg(sk, rs);
		copa->last_report_time = now;
		copa->report_acked_bytes = 0;
		copa->report_lost_bytes = 0;
		minmax_reset_u64(&stg->standing_rtt, now, 1 * USEC_PER_SEC);
		tp->snd_ssthresh = tp->snd_cwnd;
	}
}

SEC("struct_ops/bpf_copa_init")
void BPF_PROG(bpf_copa_init, struct sock *sk)
{
	struct tcp_sock *tp = tcp_sk(sk);
	struct copa *copa = inet_csk_ca(sk);
	tp->snd_ssthresh = TCP_INFINITE_SSTHRESH;
	// minmax_reset_u64(&copa->min_rtt, min_rtt_window, 0);
	// minmax_reset_u64(&copa->standing_rtt, standing_rtt_window, 0);
	copa->use_standing_rtt = true;
	copa->is_slow_start = true;
	copa->delta_param = 40;
	copa->default_param = 40;
	copa->last_cwnd_double_time = 0;
	// velocity_reset(&copa->velocity_state);
	copa->ack_bytes_round = 0;
	// copa_init_ringbuf(&copa->rtt_ringbuf);
	// copa_init_ringbuf(&copa->increase_ringbuf);
	copa->max_time = 10 * USEC_PER_SEC;
	copa->num_increase = 0;
	copa->mode = Default;
	copa->mode_switch = true;
	copa->total_acked_bytes = 0;
	copa->cur_num_acked = 0;
	copa->cur_num_losses = 0;
	copa->report_acked_bytes = 0;
	copa->report_lost_bytes = 0;
	copa->prev_loss_cycle = 0;
	copa->prev_loss_rate = 0;
	copa->last_report_time = 0;
	copa->have_flow_id = false;
	copa->flow_id = 0;
	copa_init_pacing_rate_from_rtt(sk, 2 << COPA_SCALE);
	cmpxchg(&sk->sk_pacing_status, SK_PACING_NONE, SK_PACING_NEEDED);
	struct copa_info *stg =
		bpf_sk_storage_get(&copa_info_stg, (void *)sk, NULL,
				   BPF_LOCAL_STORAGE_GET_F_CREATE);
	if (stg) {
		stg->last_report_timestamp = tcp_clock_us();
		stg->first_timestamp = tcp_clock_us();
		minmax_reset_u64(&stg->min_rtt, min_rtt_window, 0);
		minmax_reset_u64(&stg->standing_rtt, standing_rtt_window, 0);
		copa_init_ringbuf(&stg->rtt_ringbuf);
		copa_init_ringbuf(&stg->increase_ringbuf);
		velocity_reset(&stg->velocity_state);
		reset_trade_off_record(&stg->trade_off_stg);
	}
}

SEC("struct_ops/bpf_copa_undo_cwnd")
u32 BPF_PROG(bpf_copa_undo_cwnd, struct sock *sk)
{
	return tcp_sk(sk)->snd_cwnd;
}

SEC("struct_ops/bpf_copa_ssthresh")
u32 BPF_PROG(bpf_copa_ssthresh, struct sock *sk)
{
	return tcp_sk(sk)->snd_ssthresh;
}

SEC(".struct_ops")
static struct tcp_congestion_ops copa = {
	.flags = TCP_CONG_NON_RESTRICTED,
	.name = "mortise_copa",
	.init = (void *)bpf_copa_init,
	.cong_control = (void *)bpf_copa_main,
	.undo_cwnd = (void *)bpf_copa_undo_cwnd,
	.ssthresh = (void *)bpf_copa_ssthresh,
};
