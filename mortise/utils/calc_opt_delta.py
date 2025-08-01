#!/usr/bin/env python3
import numpy as np
from utils.change_point import build_detector
from utils.signal_process import *
from utils.calc_copa_performance import *
import struct
import time

# Sup Param
LOSS_THR = 5e-3

# Initial value
# Tput: Mbps, RTT: Secs, loss: loss_rate
# Response size: Mb, chunk size: Mb, playback buffer size: Mb
LAMBDA = 0.1
BETA = 0.1
RESPONSE_SIZE = 4.87
# example
CHUNK_SIZE = 2

ALPHA = 0.6
CP = 1
STEP_EPS = 0.24
MOVE_STEP_EPS = STEP_EPS


class AppInfo:
    def __init__(self, trade_off):
        self.req = trade_off
        self.resp = 0

    def struct_to_u8_array(self):
        return list(struct.pack("<QQ", self.req, self.resp))


class ReportEntry:
    flow_id = 0
    chunk_id = 0
    chunk_len = 0
    data_array = []


class FlowCtrl:
    def __init__(self, app="STREAMING"):
        self.cp_detector = build_detector()
        self.cp_detected = False
        self.run_len = np.inf
        self.last_run_len = np.inf
        self.history_rtt = np.empty(0)
        self.history_acked_bytes = np.empty(0)
        self.history_lost_bytes = np.empty(0)
        self.history_timestamp = np.empty(0)
        self.history_max_qlen = np.empty(0)
        self.smoothed_rate = np.empty(0)
        self.smoothed_bdp = np.empty(0)
        self.ewma_rate = 0
        self.loss_rate = 0.0
        self.minrtts = np.empty(0)
        self.minrtt_timestamp = np.empty(0)
        self.sample_interval = 0.01
        self.cur_trade_off = 100
        self.last_trade_off = 100
        self.intervals_len = np.zeros(1)
        self.duration_begin = 0.0
        self.enable_adjust = False
        self.flow_id = None
        self.decide_intervals_cnts = 0
        self.start_time = time.time()
        self.app_type = app

        self.qoe_lambda = LAMBDA
        self.qoe_beta = BETA

    def clear_history(self):
        self.history_rtt = np.empty(0)
        self.history_acked_bytes = np.empty(0)
        self.history_lost_bytes = np.empty(0)
        self.history_timestamp = np.empty(0)
        self.history_max_qlen = np.empty(0)
        self.smoothed_rate = np.empty(0)
        self.smoothed_bdp = np.empty(0)
        self.ewma_rate = 0
        self.decide_intervals_cnts = 0
        # self.cur_trade_off = 200

    def update_loss(self):
        # most recent 2 intervals, about 10 rtts
        start_index = -int(np.sum(self.intervals_len[-2:]))
        total_acked = np.sum(self.history_acked_bytes[start_index:])
        total_lost = np.sum(self.history_lost_bytes[start_index:])
        self.loss_rate = total_lost / (total_lost + total_acked)
        if self.loss_rate > 0:
            self.history_max_qlen = np.append(
                self.history_max_qlen,
                (1 - self.loss_rate) / self.cur_trade_off * 1000.0,
            )

    def update_minrtt(self, cur_min_rtt):
        self.minrtt_timestamp = np.append(self.minrtt_timestamp, time.time())
        self.minrtts = np.append(self.minrtts, cur_min_rtt)
        while (
            len(self.minrtt_timestamp) > 0
            and time.time() - self.minrtt_timestamp[0] > 10.0
        ):
            self.minrtt_timestamp = self.minrtt_timestamp[1:]
            self.minrtts = self.minrtts[1:]

    # Note: we need to know about the current qoe preference in real-time
    def update_qoe_preference(self, tput, delay, loss_rate):
        if loss_rate < 0.05:
            (a, b) = (0, 0)
        elif loss_rate < 0.1:
            (a, b) = (4, -0.2)
        elif loss_rate < 0.4:
            (a, b) = (1, 0.1)
        else:
            (a, b) = (0, 0.5)

        if self.app_type == "FILE":
            # P(r) = a r + b
            delay /= 1000.0
            response = RESPONSE_SIZE
            # Note: lambda transfer to Mbps/ms
            self.qoe_lambda = (tput * tput) * (loss_rate + 2) / (2 * response) / 1000.0
            self.qoe_beta = (
                -tput
                * (2 * a * (response + tput * delay) - (b - 1) * tput * delay)
                / (2 * response * (a * loss_rate + b - 1))
            )
            # print(f"lambda {self.qoe_lambda} beta {self.qoe_beta}")
        elif self.app_type == "STREAMING":
            self.qoe_lambda = (
                2.66
                * (tput * tput)
                * (loss_rate + 2)
                / (tput + CHUNK_SIZE * 2.66)
                / 1000.0
            )
            self.qoe_beta = 2.66 * (tput * tput) * delay / (tput + CHUNK_SIZE * 2.66)

    # using sliding window to calculate the smoothed_bdp and ewma rate
    def update_smoothed_data(self, timestamps, bytes, rtts):
        rtt_min = np.min(rtts) / 1000.0
        wnd_len = rtt_min
        # Note: currently we do not support ultra low latency scenario like datacenter
        self.sample_interval = max(0.004, rtt_min / 4)
        step = self.sample_interval

        # rate in B/s
        raw_rates = sliding_window_rate(timestamps, bytes, rtts, step, wnd_len)
        raw_rate_mbps = raw_rates * 8 / 1024 / 1024

        self.ewma_rate = update_ewma(self.ewma_rate, raw_rate_mbps)
        self.smoothed_rate = np.append(self.smoothed_rate, raw_rate_mbps)
        self.smoothed_bdp = np.append(self.smoothed_bdp, raw_rates * rtt_min / 1448)
        # print(f"rate: {np.mean(raw_rates)}, ewma-rate: {self.ewma_rate}")

    # we need to compare the app qoe preference and network tradeoff slope
    # thus get the corresponding network 'lambda' and 'beta' first
    def get_net_lambda_beta(self):
        start_index = -int(np.sum(self.intervals_len[-4:]))
        # start_index = -int(np.sum(self.intervals_len[-(self.decide_intervals_cnts - 2):]))
        rtt_min = np.min(self.minrtts)
        bdp_start_index = -int(
            (self.history_timestamp[-1] - self.history_timestamp[start_index])
            / self.sample_interval
        )
        bdp_start_index = max(bdp_start_index, -int(len(self.smoothed_bdp)))
        bdp = self.smoothed_bdp[bdp_start_index:]
        bw = self.ewma_rate
        # Nyquist sample rule
        # high-pass, bandpass filter
        # copa 's delay win is half min_rtt
        cutoff = int(1000 / (2 * (1 + 0.5) * rtt_min))
        # make sure  0 < wn < 1 when the sample interval was not initialized
        fs = max(1 / self.sample_interval, 2.01 * cutoff)
        rp = 1
        bdp_h = cheby_highpass_filter(bdp, cutoff, fs, rp, order=4)
        bdp_zd = bdp - np.mean(bdp)
        bdp_b = cheby_lowpass_filter(bdp_zd, cutoff, fs, rp, order=2)
        bdp_p2p_h = 2 * np.std(bdp_h)
        bdp_p2p_l = 2 * np.std(bdp_b)
        peak_width_l = compute_average_peak_width(bdp_b) * self.sample_interval

        delta_large = min(500, self.cur_trade_off * (1 + STEP_EPS))
        delta_small = max(
            int(self.cur_trade_off / 3), self.cur_trade_off * (1.0 - STEP_EPS)
        )
        tput_h = calc_relative_tput_high_freq(
            bdp_p2p_h, delta_small, rtt_min / 1000.0
        ) - calc_relative_tput_high_freq(bdp_p2p_h, delta_large, rtt_min / 1000.0)
        tput_l = calc_relative_tput_low_freq(
            bdp_p2p_l, delta_small, rtt_min / 1000.0, peak_width_l
        ) - calc_relative_tput_low_freq(
            bdp_p2p_l, delta_large, rtt_min / 1000.0, peak_width_l
        )
        thr = tput_h + tput_l
        lat_mean = calc_queue_delay(
            delta_small, bw, rtt_min, bounce=True
        ) - calc_queue_delay(delta_large, bw, rtt_min, bounce=True)
        if len(self.history_max_qlen) > 0:
            max_qlen = np.mean(self.history_max_qlen)
            loss_mean = calc_loss(delta_small, max_qlen) - calc_loss(
                delta_large, max_qlen
            )
        else:
            loss_mean = 0
        beta = 0

        # avoid /0 error
        if loss_mean > LOSS_THR:
            beta = thr / loss_mean
        return (thr / lat_mean, beta)

    def probe_opt_delta(self):
        rtt_min = np.min(self.minrtts)
        (net_lambda, net_beta) = self.get_net_lambda_beta()

        # Beta adjustment logic
        beta_opt_d = self.cur_trade_off
        if net_beta < self.qoe_beta and net_beta > 0:
            beta_opt_d *= 1.0 + MOVE_STEP_EPS
        else:
            beta_opt_d = 0

        # Lambda adjustment strategy: choose different methods based on proximity to target
        lambda_ratio = net_lambda / self.qoe_lambda if self.qoe_lambda > 0 else 0
        if 0.5 < lambda_ratio < 2:
            # Lambda close to target: use filtering + search for fine-grained adjustment
            # print(f"Lambda close to target ({lambda_ratio:.2f}), using filtering fine-tuning")
            return self._fine_tune_with_filtering(rtt_min, beta_opt_d)
        else:
            # Lambda far from target: use direct stepping movement
            # print(f"Lambda far from target ({lambda_ratio:.2f}), using stepping adjustment")
            return self._coarse_adjust_with_stepping(net_lambda, beta_opt_d)

    def _fine_tune_with_filtering(self, rtt_min, beta_opt_d):
        """Fine-grained adjustment using filtering"""
        # BDP feature extraction
        start_index = -int(np.sum(self.intervals_len[-4:]))
        bdp_start_index = -int(
            (self.history_timestamp[-1] - self.history_timestamp[start_index])
            / self.sample_interval
        )
        bdp_start_index = max(bdp_start_index, -int(len(self.smoothed_bdp)))
        bdp = self.smoothed_bdp[bdp_start_index:]
        bw = self.ewma_rate

        # Filtering processing
        # make sure  0 < wn < 1 when the sample interval was not initialized
        cutoff = int(1000 / (2 * 1.5 * rtt_min))
        fs = max(1 / self.sample_interval, 2.01 * cutoff)
        bdp_h = cheby_highpass_filter(bdp, cutoff, fs, rp=0.8, order=4)
        bdp_zd = bdp - np.mean(bdp)
        bdp_b = cheby_lowpass_filter(bdp_zd, cutoff, fs, rp=0.8, order=2)
        bdp_p2p_h = 2 * np.std(bdp_h)
        bdp_p2p_l = 2 * np.std(bdp_b)
        peak_width_l = max(
            compute_average_peak_width(bdp_b) * self.sample_interval, 1 / cutoff
        )

        # Search for optimal delta
        opt_delta = self.cur_trade_off
        opt_qoe = -1000000

        # Calculate search range
        delay_thr = 0.08 * rtt_min
        delta_max = min(int(12 / delay_thr / self.ewma_rate * 1000), 500)
        delta_min = max(12 + int(100 * self.qoe_lambda), int(self.cur_trade_off / 2))
        # Use current weights for QoE optimization search

        for delta in range(delta_min, delta_max, 25):
            tput_h = calc_relative_tput_high_freq(bdp_p2p_h, delta, rtt_min / 1000.0)
            tput_l = calc_relative_tput_low_freq(
                bdp_p2p_l, delta, rtt_min / 1000.0, peak_width_l
            )
            thr = tput_h + tput_l
            lat_mean = calc_queue_delay(delta, bw, rtt_min, bounce=True)

            if len(self.history_max_qlen) > 0:
                max_qlen = np.mean(self.history_max_qlen)
            else:
                max_qlen = 0xFFFFFFF
            loss_mean = calc_loss(delta, max_qlen)

            # Calculate QoE using current lambda and adjusted beta
            cur_qoe = (
                thr
                - self.qoe_lambda * lat_mean / (1.0 - loss_mean)
                - self.qoe_beta * loss_mean
            )
            if cur_qoe > opt_qoe:
                opt_delta = delta
                opt_qoe = cur_qoe

        # Post-processing fine-tuning
        if opt_delta <= self.cur_trade_off and self.loss_rate < LOSS_THR:
            opt_delta = self.cur_trade_off - self.cur_trade_off * 0.1 / (
                0.2 + self.qoe_lambda + self.qoe_beta
            )
            opt_delta = max(opt_delta, delta_min)
        if self.loss_rate > LOSS_THR:
            opt_delta += self.cur_trade_off * min(
                0.5, self.qoe_beta * self.loss_rate * 12
            )

        return opt_delta

    def _coarse_adjust_with_stepping(self, net_lambda, beta_opt_d):
        """Coarse adjustment using stepping"""
        lambda_opt_d = self.cur_trade_off

        # Lambda stepping adjustment
        if net_lambda < self.qoe_lambda:
            # print(f"Lambda too low, increasing: {self.cur_trade_off} -> {lambda_opt_d}")
            # Additional adjustment based on loss_rate
            if self.loss_rate < LOSS_THR:
                # Low loss rate, can appropriately reduce sensitivity
                lambda_opt_d *= 1.0 + MOVE_STEP_EPS / 2
            else:
                lambda_opt_d *= 1.0 + MOVE_STEP_EPS
        else:
            if self.loss_rate > LOSS_THR:
                # Has loss, increase sensitivity based on beta weight
                lambda_opt_d /= 1.0 + MOVE_STEP_EPS / 2
            else:
                lambda_opt_d /= 1.0 + MOVE_STEP_EPS
            # print(f"Lambda too high, decreasing: {self.cur_trade_off} -> {lambda_opt_d}")

        # Ensure within reasonable range
        delta_min = max(10 + int(100 * self.qoe_lambda), int(self.cur_trade_off / 3))
        rtt_min = np.min(self.minrtts)
        delay_thr = 0.1 * rtt_min
        delta_max = min(int(12 / delay_thr / self.ewma_rate * 1000), 500)

        lambda_opt_d = max(min(lambda_opt_d, delta_max), delta_min)

        return lambda_opt_d

    def check_change_point(self):
        cur_run_len = self.cp_detector.add_data(self.ewma_rate)
        # print(f"cur {cur_run_len} prev len {self.run_len} prob {self.cp_detector.get_prob(cur_run_len)} rate {self.ewma_rate}")
        # double check
        if self.run_len < cur_run_len < self.last_run_len and cur_run_len <= 10:
            # print(
            #     f"detected at {time.time() - self.start_time} cur: {cur_run_len} prev len: {self.run_len} prevprev {self.last_run_len} rate {self.ewma_rate}")
            self.cp_detected = True
        self.last_run_len = self.run_len
        self.run_len = cur_run_len

    def add_data(self, report_entry: ReportEntry):
        if self.flow_id is None:
            self.flow_id = report_entry.flow_id
        chunk_len = report_entry.chunk_len
        chunk_id = report_entry.chunk_id
        self.intervals_len[-1] = self.intervals_len[-1] + chunk_len
        if chunk_id < 0:
            # end of the interval
            self.intervals_len = np.append(self.intervals_len, 0)
        if chunk_len == 0:
            return

        times = [
            float(elem.timestamp) / 1_000_000.0
            for elem in report_entry.data_array[:chunk_len]
        ]
        rtts = [
            float(elem.rtt) / 1000.0 for elem in report_entry.data_array[:chunk_len]
        ]
        bytes = [
            float(elem.acked_bytes) for elem in report_entry.data_array[:chunk_len]
        ]
        losts = [float(elem.lost_bytes) for elem in report_entry.data_array[:chunk_len]]

        # smoothed bytes
        if len(self.history_rtt) > 0:
            wnd_len = np.min(self.minrtts) / 1000.0
            index = 0
            for index in range(1, len(self.history_timestamp[:])):
                if self.history_timestamp[-index] + wnd_len < times[0]:
                    break
            index = -index
            combined_times = np.append(self.history_timestamp[index:], times)
            combined_rtts = np.append(self.history_rtt[index:], rtts)
            combined_bytes = np.append(self.history_acked_bytes[index:], bytes)
            self.update_smoothed_data(combined_times, combined_bytes, combined_rtts)

        # print(f"at {time.time() - self.start_time} recieve chunk len: {chunk_len} with rate {self.ewma_rate} with intervals cnt {self.decide_intervals_cnts}")
        self.history_rtt = np.append(self.history_rtt, rtts)
        self.history_timestamp = np.append(self.history_timestamp, times)
        self.history_acked_bytes = np.append(self.history_acked_bytes, bytes)
        self.history_lost_bytes = np.append(self.history_lost_bytes, losts)

        # update minrtt in ms and loss rate
        self.update_minrtt(np.min(rtts))
        self.update_loss()
        # update qoe preference
        self.update_qoe_preference(
            self.ewma_rate, np.min(self.minrtts) / 1000.0, self.loss_rate
        )
        # update cp detector
        if CP == 1:
            self.check_change_point()

        # end of interval
        if chunk_id < 0:
            self.enable_adjust = True
            self.decide_intervals_cnts += 1

    def process(self) -> dict:
        message_dict = None
        alpha = ALPHA
        if self.enable_adjust:
            # every 5 intervals, update the trade-off
            if self.decide_intervals_cnts % 5 == 0 or (
                self.decide_intervals_cnts > 10 and self.cp_detected
            ):
                self.last_trade_off = self.cur_trade_off
                opt_delta = int(self.probe_opt_delta())
                if self.cp_detected:
                    # change point detected, directly move
                    self.cur_trade_off = opt_delta
                    # clear history
                    self.clear_history()
                    self.cp_detected = False
                else:
                    self.cur_trade_off = int(
                        alpha * opt_delta + (1 - alpha) * self.cur_trade_off
                    )
                app_info = AppInfo(self.cur_trade_off)
                message_dict = {
                    "Flow": {
                        "flow_id": self.flow_id,
                        "op": {
                            "SkStgMapUpdate": {
                                "map_name": "sk_stg_map",
                                "val": app_info.struct_to_u8_array(),
                                "flag": 0,
                            }
                        },
                    }
                }
        self.enable_adjust = False
        return message_dict
