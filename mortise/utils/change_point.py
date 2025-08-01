import numpy as np
from functools import partial
from utils.online_likelihood import StudentT


def constant_hazard(lam, r):
    """
    Hazard function for bayesian online learning
    Arguments:
        lam - inital prob
        r - R matrix
    """
    return 1 / lam * np.ones(r.shape)


def build_detector():
    hazard_function = partial(constant_hazard, 250)
    log_likelihood_class = StudentT(alpha=0.1, beta=0.01, kappa=1, mu=0)
    return OnlineChangePointDetection(hazard_function, log_likelihood_class)


class OnlineChangePointDetection:
    def __init__(self, hazard_function, log_likelihood_class, wnd_len=10):
        self.hazard_function = hazard_function
        self.log_likelihood_class = log_likelihood_class
        self.t0 = 0
        self.t = -1
        self.history_len = 200
        self.R = np.zeros(self.history_len + 2)
        self.R[0] = 1.0

    def add_data(self, x):
        self.t += 1
        if (self.t - self.t0) > self.history_len:
            self.prune(self.t - self.history_len)
        t = self.t - self.t0
        # Evaluate the predictive distribution for the new datum under each of
        # the parameters. This is the standard thing from Bayesian inference.
        pred_probs = self.log_likelihood_class.pdf(x)

        # Evaluate the hazard function for this interval
        H = self.hazard_function(np.array(range(t + 1)))

        # Evaluate the probability that there *was* a changepoint and we're
        # accumulating the mass back down at r = 0.
        cp_prob = np.sum(self.R[0 : t + 1] * pred_probs * H)

        # Evaluate the growth probabilities - shift the probabilities down and to
        # the right, scaled by the hazard function and the predictive
        # probabilities.
        self.R[1 : t + 2] = self.R[0 : t + 1] * pred_probs * (1 - H)
        # Put back changepoint probability
        self.R[0] = cp_prob

        # Renormalize the run length probabilities for improved numerical
        # stability.
        self.R[0 : t + 2] = self.R[0 : t + 2] / np.sum(self.R[0 : t + 2])

        # Update the parameter sets for each possible run length.
        self.log_likelihood_class.update_theta(x)

        # self.maxes[t] = self.R[:, t].argmax()
        return self.get_max()

    def get_prob(self, wnd_len):
        return self.R[wnd_len]

    def get_max(self):
        return self.R[:].argmax()

    def get_len(self):
        return self.t + 1

    def prune(self, t0):
        self.t0 = t0
        self.log_likelihood_class.prune(self.t - t0)
