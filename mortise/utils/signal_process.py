from scipy.signal import lfilter, cheby1, find_peaks_cwt, peak_widths
import numpy as np


def cheby_lowpass(cutoff, fs, rp, order=5):
    nyquist = 0.5 * fs
    normal_cutoff = cutoff / nyquist
    b, a = cheby1(order, rp, normal_cutoff, btype="low", analog=False)
    return b, a


def cheby_lowpass_filter(data, cutoff, fs, rp, order=5):
    b, a = cheby_lowpass(cutoff, fs, rp, order=order)
    y = lfilter(b, a, data)
    return y


def cheby_highpass(cutoff, fs, rp, order=5):
    nyquist = 0.5 * fs
    normal_cutoff = cutoff / nyquist
    b, a = cheby1(order, rp, normal_cutoff, btype="high", analog=False)
    return b, a


def cheby_highpass_filter(data, cutoff: float, fs, rp, order=5):
    b, a = cheby_highpass(cutoff, fs, rp, order=order)
    y = lfilter(b, a, data)
    return y


def cheby_bandpass(l_thr, h_thr, fs, rp, order=5):
    nyquist = 0.5 * fs
    normal_cutoff = [l_thr / nyquist, h_thr / nyquist]
    b, a = cheby1(order, rp, normal_cutoff, btype="bandpass", analog=False)
    return b, a


def cheby_bandpass_filter(data, l_thr, h_thr, fs, rp, order=5):
    b, a = cheby_bandpass(l_thr, h_thr, fs, rp, order=order)
    y = lfilter(b, a, data)
    return y


def compute_average_peak_width(signal):
    """Simplified version of peak width computation"""
    if len(signal) < 5:
        return 1.0

    try:
        # Use multi-width CWT
        widths = np.arange(0.7, 2.5, 0.3)
        max_peaks = find_peaks_cwt(signal, widths)

        if len(max_peaks) == 0:
            return 1.0

        # Suppress warnings and compute peak widths
        import warnings

        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            peak_width_result = peak_widths(signal, max_peaks)

        # Filter valid widths and return median
        valid_widths = peak_width_result[0][peak_width_result[0] > 0.1]

        if len(valid_widths) == 0:
            return 1.0

        return float(np.clip(np.median(valid_widths), 0.1, len(signal) / 3))

    except:
        return 1.0


def sliding_window_rate(times, vals, rtts, step=0.005, window_length=0.02):
    result_values = np.array([])
    wnd_start_time = times[0]
    left_idx = 0
    right_idx = 0

    while right_idx < len(times):
        wnd_start_time = wnd_start_time + step
        wnd_end_time = wnd_start_time + window_length
        while times[left_idx] < wnd_start_time:
            left_idx += 1
            if left_idx >= len(times):
                break
        while times[right_idx] < wnd_end_time:
            if right_idx < len(times) - 1:
                if times[right_idx + 1] - times[right_idx] > window_length / 2:
                    if (rtts[right_idx + 1] - rtts[right_idx]) / 1000.0 < 0.5 * (
                        times[right_idx + 1] - times[right_idx]
                    ):
                        # print("APP Limit ", times[right_idx+1], times[right_idx], rtts[right_idx+1], rtts[right_idx])
                        padding = (times[right_idx + 1] - times[right_idx]) * 0.9
                        wnd_start_time += padding
                        wnd_end_time += padding
            right_idx += 1
            if right_idx >= len(times):
                break
        if left_idx < right_idx and window_length > 0:
            result_values = np.append(
                result_values, np.sum(vals[left_idx:right_idx]) / window_length
            )

    return result_values[:-1]


def update_ewma(old_value, new_samples, max_wnd_len=20):
    ewma_wnd_len = min(len(new_samples), max_wnd_len)
    ewma_coeff = np.power(0.8, np.arange(0, ewma_wnd_len)[::-1]) * 0.2
    new_value = np.dot(ewma_coeff, new_samples[-ewma_wnd_len:]) + old_value * np.power(
        0.8, ewma_wnd_len
    )
    return new_value
