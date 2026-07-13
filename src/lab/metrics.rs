//! Deterministic quality metrics used by fixture and external-corpus oracles.

/// Binary classification confusion matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConfusionMatrix {
    /// Expected and observed positives.
    pub true_positive: usize,
    /// Expected negatives observed as positive.
    pub false_positive: usize,
    /// Expected positives observed as negative.
    pub false_negative: usize,
    /// Expected and observed negatives.
    pub true_negative: usize,
}

impl ConfusionMatrix {
    /// Precision, treating an empty predicted-positive set as perfect only
    /// when no positives were expected.
    pub fn precision(self) -> f64 {
        ratio(
            self.true_positive,
            self.true_positive + self.false_positive,
            self.false_negative == 0,
        )
    }

    /// Recall, treating an empty expected-positive set as perfect.
    pub fn recall(self) -> f64 {
        ratio(
            self.true_positive,
            self.true_positive + self.false_negative,
            true,
        )
    }
}

/// Mean reciprocal rank for one-based optional ranks.
pub fn mean_reciprocal_rank(ranks: &[Option<usize>]) -> f64 {
    if ranks.is_empty() {
        return 1.0;
    }
    ranks
        .iter()
        .map(|rank| rank.map_or(0.0, |value| 1.0 / value.max(1) as f64))
        .sum::<f64>()
        / ranks.len() as f64
}

/// Normalized discounted cumulative gain for binary relevance.
pub fn ndcg(relevance: &[bool]) -> f64 {
    if relevance.is_empty() {
        return 1.0;
    }
    let dcg = relevance
        .iter()
        .enumerate()
        .filter(|(_, relevant)| **relevant)
        .map(|(index, _)| 1.0 / ((index + 2) as f64).log2())
        .sum::<f64>();
    let relevant = relevance.iter().filter(|value| **value).count();
    if relevant == 0 {
        return 1.0;
    }
    let ideal = (0..relevant)
        .map(|index| 1.0 / ((index + 2) as f64).log2())
        .sum::<f64>();
    dcg / ideal
}

/// Adjusted Rand Index over two equal-length cluster assignments.
pub fn adjusted_rand_index(expected: &[usize], observed: &[usize]) -> Option<f64> {
    if expected.len() != observed.len() || expected.len() < 2 {
        return None;
    }
    let mut contingency = std::collections::BTreeMap::<(usize, usize), usize>::new();
    let mut expected_counts = std::collections::BTreeMap::<usize, usize>::new();
    let mut observed_counts = std::collections::BTreeMap::<usize, usize>::new();
    for (&left, &right) in expected.iter().zip(observed) {
        *contingency.entry((left, right)).or_default() += 1;
        *expected_counts.entry(left).or_default() += 1;
        *observed_counts.entry(right).or_default() += 1;
    }
    let pairs = |count: usize| count.saturating_mul(count.saturating_sub(1)) / 2;
    let sum_cells = contingency.values().copied().map(pairs).sum::<usize>() as f64;
    let sum_expected = expected_counts.values().copied().map(pairs).sum::<usize>() as f64;
    let sum_observed = observed_counts.values().copied().map(pairs).sum::<usize>() as f64;
    let total = pairs(expected.len()) as f64;
    if total == 0.0 {
        return None;
    }
    let chance = sum_expected * sum_observed / total;
    let maximum = (sum_expected + sum_observed) / 2.0;
    Some(if (maximum - chance).abs() < f64::EPSILON {
        1.0
    } else {
        (sum_cells - chance) / (maximum - chance)
    })
}

/// Normalized mutual information over two equal-length cluster assignments.
pub fn normalized_mutual_information(expected: &[usize], observed: &[usize]) -> Option<f64> {
    if expected.len() != observed.len() || expected.is_empty() {
        return None;
    }
    let mut contingency = std::collections::BTreeMap::<(usize, usize), usize>::new();
    let mut expected_counts = std::collections::BTreeMap::<usize, usize>::new();
    let mut observed_counts = std::collections::BTreeMap::<usize, usize>::new();
    for (&left, &right) in expected.iter().zip(observed) {
        *contingency.entry((left, right)).or_default() += 1;
        *expected_counts.entry(left).or_default() += 1;
        *observed_counts.entry(right).or_default() += 1;
    }
    let total = expected.len() as f64;
    let entropy = |counts: &std::collections::BTreeMap<usize, usize>| {
        counts
            .values()
            .map(|count| *count as f64 / total)
            .filter(|probability| *probability > 0.0)
            .map(|probability| -probability * probability.ln())
            .sum::<f64>()
    };
    let expected_entropy = entropy(&expected_counts);
    let observed_entropy = entropy(&observed_counts);
    let denominator = (expected_entropy * observed_entropy).sqrt();
    if denominator <= f64::EPSILON {
        return Some(f64::from(expected == observed));
    }
    let mutual_information = contingency
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|((left, right), count)| {
            let joint = *count as f64 / total;
            let left_probability = expected_counts[left] as f64 / total;
            let right_probability = observed_counts[right] as f64 / total;
            joint * (joint / (left_probability * right_probability)).ln()
        })
        .sum::<f64>();
    Some((mutual_information / denominator).clamp(0.0, 1.0))
}

fn ratio(numerator: usize, denominator: usize, empty_is_one: bool) -> f64 {
    if denominator == 0 {
        f64::from(empty_is_one)
    } else {
        numerator as f64 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_cover_perfect_partial_and_empty_inputs() {
        let perfect = ConfusionMatrix {
            true_positive: 2,
            true_negative: 1,
            ..ConfusionMatrix::default()
        };
        assert_eq!(perfect.precision(), 1.0);
        assert_eq!(perfect.recall(), 1.0);
        assert_eq!(mean_reciprocal_rank(&[Some(1), Some(2)]), 0.75);
        assert_eq!(mean_reciprocal_rank(&[]), 1.0);
        assert_eq!(ndcg(&[true, false]), 1.0);
        assert_eq!(adjusted_rand_index(&[0, 0, 1, 1], &[7, 7, 4, 4]), Some(1.0));
        assert_eq!(adjusted_rand_index(&[0], &[0]), None);
        assert_eq!(
            normalized_mutual_information(&[0, 0, 1, 1], &[7, 7, 4, 4]),
            Some(1.0)
        );
        assert_eq!(normalized_mutual_information(&[], &[]), None);
    }
}
