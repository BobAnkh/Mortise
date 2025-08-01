import math

USE_OLD = False

USE_OLD = False


# calculate the relative throughput relative to full utilization
def calc_relative_tput_high_freq(p2p, delta_scaled, minrtt=0.06):
    delta = delta_scaled / 1000.0
    height = p2p / 2
    extra_cwnd = min(height, 1 / delta)
    rel_tput_in_packets = -((height - extra_cwnd) ** 2) / (2 * max(2, height) * minrtt)
    return rel_tput_in_packets * 1448 * 8 / 1024.0 / 1024.0


# calculate the relative throughput relative to full utilization
def calc_relative_tput_low_freq(p2p, delta_scaled, minrtt=0.06, avg_peak_width=0.32):
    delta = delta_scaled / 1000.0
    delta_packets = 0
    cur_cwnd = 0
    # due to the special bounce mechanisms(no drain queue), if delta < 100, the init cwnd should be...
    if delta <= 0.1:
        cur_cwnd += 0.5 / delta
    cur_time = 0
    round_cnt = 0
    while cur_cwnd < p2p and round_cnt < 6 and cur_time < avg_peak_width:
        cur_cwnd += 0.5 / delta
        round_cnt += 1
        cur_time += 0.5 * minrtt
        delta_packets += max(0.5 * (p2p - cur_cwnd), 0)
    delta_p2p = p2p - cur_cwnd
    if delta_p2p >= 1 and cur_time < avg_peak_width:
        max_converge_rounds = int((avg_peak_width - cur_time) * 2 / minrtt)
        converge_rounds = min(
            math.ceil(math.log2(2 * delta * delta_p2p + 1)), max_converge_rounds
        )
        delta_packets += (
            converge_rounds * delta_p2p / 2
            - (2**converge_rounds - 2 - converge_rounds) / 4 / delta
        )
    return -(delta_packets / avg_peak_width) * 12 / 1000.0


# calculate the average queueing latency
def calc_queue_delay(delta_scaled, bandwidth, minrtt=0.06, bounce=False):
    delta = delta_scaled / 1000.0
    # avoid /0 error
    if bandwidth == 0.0:
        bandwidth = 0.001
    delay = 1.25 * 12 / delta / bandwidth
    # due to the special bounce mechanisms(no drain queue), if delta < 100, the average latency should be around 1.3x
    if delta <= 0.1 and bounce:
        delay = 1.3 * delay
    return delay


# calculate the possible loss rate
def calc_loss(delta_scaled, max_qlen):
    # delta determines the average queue length which determines the losses
    return max(0.0, 1.0 - max_qlen * delta_scaled / 1000.0)
