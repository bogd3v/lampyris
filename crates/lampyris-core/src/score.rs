//! Explainable coordination score.
//!
//! The score is a transparent heuristic, **not** a calibrated probability. Every
//! report ships the raw [`Signals`] next to it so that anyone can recompute it,
//! disagree with it, or plug in a better model. Bump [`SCORE_MODEL_VERSION`]
//! whenever the formula changes so reports stay comparable.

use serde::{Deserialize, Serialize};

pub const SCORE_MODEL_VERSION: &str = "heuristic-v0";

/// Raw, verifiable measurements of a cluster of near-identical posts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signals {
    /// Distinct accounts that posted the (near-)same text.
    pub accounts: usize,
    /// Total posts in the cluster (an account can post several times).
    pub posts: usize,
    /// Seconds between the first and the last post.
    pub span_secs: f64,
    /// Distinct accounts per minute over the span (a minimum span of one minute
    /// is assumed so a single burst does not divide by zero).
    pub accounts_per_minute: f64,
    /// Mean Hamming distance of each post to the cluster's first post (0..=64).
    /// 0 means every copy normalizes to exactly the same text.
    pub mean_distance: f64,
    /// Share of posts that embed an external link card (0..=1). High values often
    /// mean people sharing the same news headline, which is usually legitimate.
    pub external_link_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// 0..=1. Higher means more consistent with coordinated activity.
    pub value: f64,
    pub model: String,
    /// The individual factors that were multiplied together, for display.
    pub factors: Vec<(String, f64)>,
}

impl Signals {
    pub fn score(&self, max_distance: u32) -> Score {
        // More distinct accounts → stronger evidence, saturating.
        let size = 1.0 - (-(self.accounts as f64) / 15.0).exp();
        // Faster → less likely to be organic. ~10 accounts/min already scores high.
        let speed = 1.0 - (-self.accounts_per_minute / 4.0).exp();
        // Exact copies weigh more than loose paraphrases.
        let sameness = 1.0 - 0.5 * (self.mean_distance / (max_distance as f64 + 1.0)).min(1.0);
        // Headline sharing is the most common legitimate cause; damp it, don't zero it.
        let link_damping = 1.0 - 0.5 * self.external_link_ratio.clamp(0.0, 1.0);

        let factors = vec![
            ("size".to_string(), size),
            ("speed".to_string(), speed),
            ("sameness".to_string(), sameness),
            ("link_damping".to_string(), link_damping),
        ];
        let value = factors
            .iter()
            .map(|(_, f)| f)
            .product::<f64>()
            .clamp(0.0, 1.0);
        Score {
            value,
            model: SCORE_MODEL_VERSION.to_string(),
            factors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(accounts: usize, per_min: f64, links: f64) -> Signals {
        Signals {
            accounts,
            posts: accounts,
            span_secs: 60.0,
            accounts_per_minute: per_min,
            mean_distance: 0.0,
            external_link_ratio: links,
        }
    }

    #[test]
    fn bigger_and_faster_scores_higher() {
        let small_slow = signals(5, 0.5, 0.0).score(3).value;
        let big_fast = signals(80, 40.0, 0.0).score(3).value;
        assert!(big_fast > small_slow);
        assert!(big_fast > 0.9);
    }

    #[test]
    fn link_shares_are_damped() {
        let plain = signals(50, 20.0, 0.0).score(3).value;
        let links = signals(50, 20.0, 1.0).score(3).value;
        assert!(links < plain);
    }
}
