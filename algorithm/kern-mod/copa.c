#include <linux/module.h>
#include <net/tcp.h>
#include <linux/inet_diag.h>
#include <linux/inet.h>
#include <linux/random.h>
#include <linux/win_minmax.h>
#include <linux/list.h>
#include <linux/slab.h>
#include <linux/module.h>
#include <linux/win_minmax.h>

/* Scale factor for rate in pkt/uSec unit to avoid truncation in bandwidth
 * estimation. The rate unit ~= (1500 bytes / 1 usec / 2^24) ~= 715 bps.
 * This handles bandwidths from 0.06pps (715bps) to 256Mpps (3Tbps) in a u32.
 * Since the minimum window is >=4 packets, the lower bound isn't
 * an issue. The upper bound isn't an issue with existing technologies.
 */
#define BW_SCALE 24
#define BW_UNIT (1 << BW_SCALE)
#define COPA_SCALE 8
#define COPA_UNIT (1 << COPA_SCALE)

#define MIM_HASH 0
#define MAX_ARRAY_SIZE 100000
// the BINARY_SEARCH_LIMIT is the result of log_2 (MAX_ARRAY_SIZE) (~=17)
#define BINARY_SEARCH_LIMIT 20

struct rtt_entry {
	u64 rtt;
	u64 time;
	struct list_head list;
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

static const u32 min_rtt_window = 10 * USEC_PER_SEC; // 10 seconds
static const u32 standing_rtt_window = 100 * USEC_PER_MSEC; // 100 ms
static const u32 coop_threshold = 100; // be careful of the bounce mechanisms
static const u64 quantization_base = 1000;
static const u64 default_delta_param = 500; // more agressive
static const u64 default_max_param = 500; // more agressive
static const u64 report_rtt_interval = 500; //quantization_base / 2;
static const u32 min_cwnd_segment = 4;
// static const u32 max_hist_time =
// 	10 *
// 	USEC_PER_SEC; // Maximum time till which to maintain history. It is minimum of 10s and 20 RTTs.
static const u32 default_bounce_intervals = 20;
static const u32 default_pacing_gain = 2;
static const u32 default_timeout_gain = 2;

struct copa {
	// u64 bitrate;
	// time in microseconds
	struct minmax min_rtt;
	struct minmax standing_rtt;
	struct minmax coop_min_rtt;
	struct minmax coop_max_rtt;
	bool is_slow_start;
	bool use_standing_rtt;
	/*
	 * delta_param determines how latency sensitive the algorithm is.
	 * Lower means it will maximize throughput at expense of delay.
	 * Higher value means it will minimize delay at expense of throughput.
	 * Default value is 500 / 1000.
	 */
	u32 delta_param : 12, default_param : 12, prev_ca_state : 8;
	u32 num_increase;
	// time at which cwnd was last doubled during slow start
	// u64 last_cwnd_double_time;
	struct list_head rtt_list_head;
	// last increase timestamp
	u64 recent_increase_time;
	enum copa_mode mode;
	u32 total_acked_bytes;
	u32 cur_num_acked;
	u32 cur_num_losses;
	u32 report_acked_bytes;
	u32 report_lost_bytes;
	// End of the last window of tracking losses
	u32 prev_loss_cycle;
	// Loss rate in the previous cycle
	u32 prev_loss_rate;
	u64 last_report_time;
	u32 prior_cwnd;
	struct copa_velocity_state velocity_state;
};

struct mortise {
	struct copa *copa;
	bool use_bounce;
	bool timeout;
	u64 last_ack_time;
	u64 next_valid_time;
	u32 bounce_intervals;
	u32 intervals_cnt;
};

/* As time advances, update the 1st, 2nd, and 3rd choices. */
static u32 minmax_subwin_update(struct minmax *m, u32 win,
				const struct minmax_sample *val)
{
	u32 dt = val->t - m->s[0].t;

	if (unlikely(dt > win)) {
		/*
	* Passed entire window without a new val so make 2nd
	* choice the new val & 3rd choice the new 2nd choice.
	* we may have to iterate this since our 2nd choice
	* may also be outside the window (we checked on entry
	* that the third choice was in the window).
	*/
		m->s[0] = m->s[1];
		m->s[1] = m->s[2];
		m->s[2] = *val;
		if (unlikely(val->t - m->s[0].t > win)) {
			m->s[0] = m->s[1];
			m->s[1] = m->s[2];
			m->s[2] = *val;
		}
	} else if (unlikely(m->s[1].t == m->s[0].t) && dt > win / 4) {
		/*
	* We've passed a quarter of the window without a new val
	* so take a 2nd choice from the 2nd quarter of the window.
	*/
		m->s[2] = m->s[1] = *val;
	} else if (unlikely(m->s[2].t == m->s[1].t) && dt > win / 2) {
		/*
	* We've passed half the window without finding a new val
	* so take a 3rd choice from the last half of the window
	*/
		m->s[2] = *val;
	}
	return m->s[0].v;
}

/* Check if new measurement updates the 1st, 2nd or 3rd choice min. */
u32 minmax_running_min(struct minmax *m, u32 win, u32 t, u32 meas)
{
	struct minmax_sample val = { .t = t, .v = meas };

	if (unlikely(val.v <= m->s[0].v) || /* found new min? */
	    unlikely(val.t - m->s[2].t > win)) /* nothing left in window? */
		return minmax_reset(m, t, meas); /* forget earlier samples */

	if (unlikely(val.v <= m->s[1].v))
		m->s[2] = m->s[1] = val;
	else if (unlikely(val.v <= m->s[2].v))
		m->s[2] = val;

	return minmax_subwin_update(m, win, &val);
}

static void velocity_reset(struct copa_velocity_state *state)
{
	state->velocity = 1;
	state->direction = None;
	state->num_times_direction_same = 0;
	state->last_recorded_cwnd_bytes = 0;
	state->last_cwnd_record_time = 0;
	state->time_since_direction = 0;
}

/* Return rate in bytes per second, optionally with a gain.
 * The order here is chosen carefully to avoid overflow of u64. This should
 * work for input rates of up to 2.9Tbit/sec and gain of 2.89x.
 */
static u64 copa_rate_bytes_per_sec(struct sock *sk, u64 rate, int gain)
{
	unsigned int mss = tcp_sk(sk)->mss_cache;

	rate *= mss;
	rate *= gain;
	rate >>= COPA_SCALE;
	rate *= USEC_PER_SEC;
	return rate >> BW_SCALE;
}

/* Convert a Copa bw and gain factor to a pacing rate in bytes per second. */
static unsigned long copa_bw_to_pacing_rate(struct sock *sk, u32 bw, int gain)
{
	u64 rate = bw;

	rate = copa_rate_bytes_per_sec(sk, rate, gain);
	rate = min_t(u64, rate, sk->sk_max_pacing_rate);
	return rate;
}

// cwnd_bytes = snd_cwnd*mss

static void copa_init_pacing_rate_from_rtt(struct sock *sk, int gain)
{
	struct tcp_sock *tp = tcp_sk(sk);
	u32 rtt_us;
	u64 bw;
	unsigned long rate;
	if (tp->srtt_us) { /* any RTT sample yet? */
		rtt_us = max(tp->srtt_us >> 3, 1U);
		bw = (u64)tp->snd_cwnd * BW_UNIT;
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

// now is in microseconds
// static void clear_old_hist(struct copa *copa, u64 now)
// {
// 	struct rtt_entry *entry, *tmp;
// 	u64 target = now - max_hist_time;
// 	if (now > max_hist_time) {
// 		list_for_each_entry_safe(entry, tmp, &copa->rtt_list_head,
// 					 list) {
// 			if (entry->time < target) {
// 				list_del(&entry->list);
// 				kfree(entry);
// 			} else {
// 				break;
// 			}
// 		}
// 	}
// }

// rtt, rtt_min, srtt and now are in microseconds
// static void new_rtt_sample(struct copa *copa, u64 rtt, u64 rtt_min, u64 srtt,
// 			   u64 now)
// {
// 	// insert rtt sample, push back data
// 	struct rtt_entry *e = kmalloc(sizeof(*e), GFP_ATOMIC);
// 	if (e) {
// 		e->rtt = rtt;
// 		e->time = now;
// 		list_add_tail(&e->list, &copa->rtt_list_head);
// 	};

// update increase (delete increase judgement, only to count)
// if (copa->num_increase == 0) {
// 	copa->recent_increase_time = now;
// 	copa->num_increase += 1;
// } else if (copa->recent_increase_time < (now - 2 * rtt_min)) {
// 	copa->recent_increase_time = now;
// 	copa->num_increase = max(40u, copa->num_increase + 1);
// }

// clear old history
// clear_old_hist(copa, now);
// }

static bool tcp_detected(struct copa *copa, u32 rtt_min, u32 srtt, u64 now)
{
	u32 min_rtt = minmax_get(&copa->coop_min_rtt);
	u32 max_rtt = minmax_get(&copa->coop_max_rtt);
	u64 thresh;
	bool res;

	thresh = rtt_min +
		 (max_rtt - rtt_min) * coop_threshold / quantization_base + 100;
	printk(KERN_INFO "recent_min: %d recent_max: %d min: %d", min_rtt,
	       max_rtt, rtt_min);

	if (min_rtt > thresh) {
		res = true;
	} else {
		res = false;
	}
	return res;
}

// ATTENTION: keep in mind that delta_param is multiplied by 1000(quantization_base)
static void report_measurement(struct copa *copa, u32 rtt_min, u32 srtt,
			       u64 now, u32 acked, u32 lost)
{
	copa->cur_num_acked += acked;
	copa->cur_num_losses += lost;
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

	if (copa->prev_loss_rate >= quantization_base / 10) {
		copa->mode = Loss;
	} else if (tcp_detected(copa, rtt_min, srtt, now)) {
		copa->mode = TCPCoop;
	} else {
		copa->mode = Default;
		copa->delta_param = default_delta_param;
	}

	if (copa->mode == Default) {
		copa->delta_param = default_delta_param;
	} else if (copa->mode == TCPCoop) {
		if (lost > 0) {
			copa->delta_param = 2 * copa->delta_param;
		} else {
			// delta = 1 / (1 + 1 / delta)
			copa->delta_param =
				copa->delta_param * quantization_base /
				(copa->delta_param + quantization_base);
		}
		if (copa->delta_param > default_max_param) {
			copa->delta_param = default_max_param;
		}
		// in case decrease to 0
		if (copa->delta_param < 7) {
			copa->delta_param = 8;
		}
		printk(KERN_INFO "[copa] delta: %d", copa->delta_param);
	} else if (copa->mode == Loss) {
		if (lost > 0) {
			copa->delta_param = 2 * copa->delta_param;
		}
		if (copa->delta_param > default_max_param) {
			copa->delta_param = default_max_param;
		}
	}
}

static void change_direction(u64 now,
			     struct copa_velocity_state *velocity_state,
			     enum copa_direction direction, u32 cwnd_bytes)
{
	if (direction == velocity_state->direction) {
		return;
	}
	velocity_state->direction = direction;
	velocity_state->velocity = 1;
	velocity_state->time_since_direction = now;
	velocity_state->last_recorded_cwnd_bytes = cwnd_bytes;
}

static void
check_and_update_direction(struct copa *copa, u64 now, u32 srtt,
			   struct copa_velocity_state *velocity_state,
			   u32 cwnd_bytes, u32 acked_bytes)
{
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

static void copa_main(struct sock *sk, const struct rate_sample *rs)
{
	struct tcp_sock *tp = tcp_sk(sk);
	struct mortise *mortise = inet_csk_ca(sk);

	struct copa *copa;
	u64 min_rtt;
	long rtt;
	u32 srtt;
	u64 now;
	u64 min_standing_rtt;

	u64 delay_us;
	u32 cwnd_bytes;
	u32 change_coef;
	u32 acked_packets;

	struct copa_velocity_state *velocity_state;
	enum copa_direction old_direction;
	bool increase_cwnd;

	u64 target_rate;
	u64 current_rate;

	u32 change;
	u32 new_cwnd;

	if (mortise->copa == NULL)
		return;

	copa = mortise->copa;
	rtt = rs->rtt_us;
	srtt = tcp_sk(sk)->srtt_us >> 3;
	now = tcp_clock_us();
	copa->report_acked_bytes += rs->acked_sacked * tp->mss_cache;
	copa->report_lost_bytes += rs->losses * tp->mss_cache;

	if (rtt >= 0) {
		minmax_running_min(&copa->min_rtt, min_rtt_window, now,
				   (u64)rtt);
		minmax_running_min(&copa->standing_rtt, (u64)srtt / 2, now,
				   (u64)rtt);
		minmax_running_min(&copa->coop_min_rtt, (u64)srtt * 6, now,
				   (u64)rtt);
		minmax_running_max(&copa->coop_max_rtt, (u64)srtt * 6, now,
				   (u64)rtt);
	}
	min_rtt = minmax_get(&copa->min_rtt);
	cwnd_bytes = tp->snd_cwnd * tp->mss_cache;

	// slow start
	if (copa->is_slow_start) {
		new_cwnd = cwnd_bytes + copa->report_acked_bytes;
		tp->snd_cwnd =
			min(new_cwnd / tp->mss_cache, tp->snd_cwnd_clamp);
		copa->report_acked_bytes = 0;
	}

	// printk(KERN_INFO "[COPA] valid %lu last_ack %lu, min_rtt %lu", mortise->next_valid_time, mortise->last_ack_time, min_rtt);
	if (!mortise->timeout && now > 0 &&
	    mortise->last_ack_time + min_rtt * default_timeout_gain < now) {
		mortise->timeout = true;
		mortise->next_valid_time = now + min_rtt; //
		printk(KERN_INFO "[COPA] timeout, next_valid_time %ld",
		       mortise->next_valid_time);
	}

	mortise->last_ack_time = now;

	if (mortise->timeout && now > 0 && now < mortise->next_valid_time) {
		return;
	}

	if (!(now > 0 && copa->last_report_time + (u64)srtt *
							  report_rtt_interval /
							  quantization_base <
				 now)) {
		return;
	}
	min_standing_rtt = minmax_get(&copa->standing_rtt);
	if (min_standing_rtt < min_rtt) {
		return;
	}

	// new_rtt_sample(copa, rtt, min_rtt, (u64)srtt, now);
	report_measurement(copa, min_rtt, srtt, now, copa->report_acked_bytes,
			   copa->report_lost_bytes);

	change_coef = 1;
	acked_packets =
		(copa->report_acked_bytes + tp->mss_cache - 1) / tp->mss_cache;
	if (copa->use_standing_rtt) {
		delay_us = min_standing_rtt - min_rtt;
	} else {
		delay_us = rtt - min_rtt;
	}

	velocity_state = &copa->velocity_state;
	old_direction = velocity_state->direction;

	if (delay_us == 0) {
		increase_cwnd = true;
	} else {
		// in bytes per second
		target_rate = tp->mss_cache * USEC_PER_SEC * quantization_base /
			      (delay_us * copa->delta_param);
		current_rate = cwnd_bytes * USEC_PER_SEC / min_standing_rtt;
		// bounce mechanisms
		if (mortise->use_bounce) {
			if (mortise->intervals_cnt %
				    mortise->bounce_intervals !=
			    0) {
				target_rate =
					target_rate * 1700 / quantization_base;
				change_coef = 2;
			}
		}

		increase_cwnd = target_rate >= current_rate;
	}

	if (!(increase_cwnd && copa->is_slow_start)) {
		check_and_update_direction(copa, now, srtt, velocity_state,
					   cwnd_bytes,
					   copa->report_acked_bytes);
	}

	change = (acked_packets * tp->mss_cache * tp->mss_cache *
		  velocity_state->velocity * quantization_base) /
		 (copa->delta_param * cwnd_bytes * change_coef);
	change = min(change, cwnd_bytes);
	if (increase_cwnd) {
		if (copa->is_slow_start) {
			// if (!copa->last_cwnd_double_time) {
			// 	copa->last_cwnd_double_time = now;
			// } else if (now - copa->last_cwnd_double_time > srtt) {
			// 	new_cwnd =
			// 		cwnd_bytes + copa->report_acked_bytes;
			// 	tp->snd_cwnd = min(new_cwnd / tp->mss_cache,
			// 			   tp->snd_cwnd_clamp);
			// 	copa->last_cwnd_double_time = now;
			// }
			// new_cwnd =
			// 	cwnd_bytes + copa->report_acked_bytes;
			// tp->snd_cwnd = min(new_cwnd / tp->mss_cache,
			// 			tp->snd_cwnd_clamp);
		} else {
			if (velocity_state->direction != Up &&
			    velocity_state->velocity > 1) {
				change_direction(now, velocity_state, Up,
						 cwnd_bytes);
			}
			new_cwnd = cwnd_bytes + change;
			tp->snd_cwnd = min(new_cwnd / tp->mss_cache,
					   tp->snd_cwnd_clamp);
		}
	} else {
		if (velocity_state->direction != Down &&
		    velocity_state->velocity > 1) {
			change_direction(now, velocity_state, Down, cwnd_bytes);
		}
		new_cwnd = cwnd_bytes - change;
		new_cwnd = max(new_cwnd, min_cwnd_segment * tp->mss_cache);
		// if (copa->is_slow_start) {
		// 	new_cwnd = min(new_cwnd, cwnd_bytes >> 1);
		// }
		tp->snd_cwnd =
			min(new_cwnd / tp->mss_cache, tp->snd_cwnd_clamp);
		copa->is_slow_start = false;
	}

	if (old_direction == Down && velocity_state->direction == Up)
		mortise->intervals_cnt += 1;

	copa_init_pacing_rate_from_rtt(sk, default_pacing_gain << COPA_SCALE);
	copa->last_report_time = now;
	copa->report_acked_bytes = 0;
	copa->report_lost_bytes = 0;
	minmax_reset(&copa->standing_rtt, now, 1 * USEC_PER_SEC);
	tp->snd_ssthresh = tp->snd_cwnd;
	/* printk(KERN_INFO "[COPA] increase: %d change %d cwnd: %d", */
	/*        increase_cwnd, change, tp->snd_cwnd); */
}

static void copa_init(struct sock *sk)
{
	struct tcp_sock *tp = tcp_sk(sk);
	struct mortise *mortise = inet_csk_ca(sk);
	tp->snd_ssthresh = TCP_INFINITE_SSTHRESH;
	mortise->use_bounce = false;
	mortise->bounce_intervals = default_bounce_intervals;
	mortise->intervals_cnt = 0;
	mortise->timeout = false;
	mortise->copa = kzalloc(sizeof(struct copa), GFP_KERNEL);
	if (mortise->copa != NULL) {
		struct copa *copa = mortise->copa;
		copa->use_standing_rtt = true;
		copa->is_slow_start = true;
		copa->delta_param = copa->default_param = default_delta_param;
		copa->mode = Default;
		copa->prev_ca_state = TCP_CA_Open;
		copa->prior_cwnd = 10;
		mortise->last_ack_time = copa->last_report_time =
			tcp_clock_us();
		// copa->last_cwnd_double_time = tcp_clock_us();
		copa->recent_increase_time = tcp_clock_us();
		minmax_reset(&copa->min_rtt, min_rtt_window, 0);
		minmax_reset(&copa->standing_rtt, standing_rtt_window, 0);
		minmax_reset(&copa->coop_min_rtt, min_rtt_window, 0);
		minmax_reset(&copa->coop_max_rtt, min_rtt_window, 0);
		INIT_LIST_HEAD(&copa->rtt_list_head);
		velocity_reset(&copa->velocity_state);
	}
	copa_init_pacing_rate_from_rtt(sk, default_pacing_gain << COPA_SCALE);
	cmpxchg(&sk->sk_pacing_status, SK_PACING_NONE, SK_PACING_NEEDED);
}

static void copa_set_state(struct sock *sk, u8 new_state)
{
	struct tcp_sock *tp = tcp_sk(sk);
	struct mortise *mortise = inet_csk_ca(sk);
	if (mortise->copa != NULL) {
		struct copa *copa = mortise->copa;
		if (new_state == TCP_CA_Loss) {
			copa->is_slow_start = true;
			copa->prev_ca_state = TCP_CA_Loss;
			/* printk(KERN_INFO "[COPA] Enter RTO"); */
		} else if (copa->prev_ca_state >= TCP_CA_Recovery &&
			   new_state < TCP_CA_Recovery) {
			/* Exiting loss recovery; restore cwnd saved before recovery. */
			u32 cwnd = max(tp->snd_cwnd, copa->prior_cwnd);
			tp->snd_cwnd = min(cwnd, tp->snd_cwnd_clamp);
		}
		copa->prev_ca_state = new_state;
	}
}

static void copa_save_cwnd(struct sock *sk)
{
	struct tcp_sock *tp = tcp_sk(sk);
	struct mortise *mortise = inet_csk_ca(sk);
	if (mortise->copa != NULL) {
		struct copa *copa = mortise->copa;
		if (copa->prev_ca_state < TCP_CA_Recovery)
			copa->prior_cwnd =
				tp->snd_cwnd; /* this cwnd is good enough */
		else /* loss recovery have temporarily cut cwnd */
			copa->prior_cwnd = max(copa->prior_cwnd, tp->snd_cwnd);
	}
}

static u32 copa_undo_cwnd(struct sock *sk)
{
	copa_save_cwnd(sk);
	return tcp_sk(sk)->snd_cwnd;
}

static u32 copa_ssthresh(struct sock *sk)
{
	return tcp_sk(sk)->snd_ssthresh;
}

static void copa_release(struct sock *sk)
{
	struct mortise *mortise = inet_csk_ca(sk);
	if (mortise->copa != NULL) {
		struct copa *copa = mortise->copa;
		struct rtt_entry *entry, *tmp;
		list_for_each_entry_safe(entry, tmp, &copa->rtt_list_head,
					 list) {
			list_del(&entry->list);
			kfree(entry);
		}
		kfree(mortise->copa);
	}
}

static struct tcp_congestion_ops tcp_copa_cong_ops __read_mostly = {
	.flags = TCP_CONG_NON_RESTRICTED,
	.name = "copa",
	.owner = THIS_MODULE,
	.init = copa_init,
	.cong_control = copa_main,
	.undo_cwnd = copa_undo_cwnd,
	.release = copa_release,
	.ssthresh = copa_ssthresh,
	.set_state = copa_set_state,
};

static int __init copa_register(void)
{
	BUILD_BUG_ON(sizeof(struct mortise) > ICSK_CA_PRIV_SIZE);
	return tcp_register_congestion_control(&tcp_copa_cong_ops);
}

static void __exit copa_unregister(void)
{
	tcp_unregister_congestion_control(&tcp_copa_cong_ops);
}

module_init(copa_register);
module_exit(copa_unregister);
MODULE_LICENSE("Dual BSD/GPL");
MODULE_DESCRIPTION("Copa MIT in Kernel Module");
