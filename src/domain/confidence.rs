//! Confidence levels for heuristically extracted facts.

use serde::{Deserialize, Serialize};

/// Confidence level for a fact an analyzer could not resolve with certainty.
///
/// Deep-language analyzers extract some facts (same-file calls, literal
/// argument values) with full certainty and others (dynamic imports,
/// computed arguments, reflection) only heuristically. Lithograph represents
/// the latter with lower confidence instead of discarding them, so the graph
/// builder and downstream documentation can show or hide them accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Confidence {
    /// Heuristic or dynamic; likely correct but not guaranteed.
    Low,
    /// Statically resolvable with certainty.
    High,
}

#[cfg(test)]
mod tests {
    use super::Confidence;

    #[test]
    fn confidence_orders_low_beneath_high() {
        assert!(Confidence::Low < Confidence::High);
    }

    #[test]
    fn confidence_serializes_as_stable_string() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(serde_json::to_string(&Confidence::High)?, "\"High\"");
        assert_eq!(serde_json::to_string(&Confidence::Low)?, "\"Low\"");

        Ok(())
    }
}
