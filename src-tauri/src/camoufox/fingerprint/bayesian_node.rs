//! Bayesian network node implementation for fingerprint generation.
//!
//! Implements weighted random sampling from conditional probability distributions.

use rand::Rng;
use serde::Deserialize;
use std::collections::HashMap;

/// Node definition from the network JSON file.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDefinition {
  pub name: String,
  pub parent_names: Vec<String>,
  pub possible_values: Vec<String>,
  pub conditional_probabilities: ConditionalProbabilities,
}

/// Conditional probability structure - can be nested or terminal.
#[derive(Debug, Clone, Deserialize)]
pub struct ConditionalProbabilities {
  #[serde(default)]
  pub deeper: Option<HashMap<String, ConditionalProbabilities>>,
  #[serde(default)]
  pub skip: Option<Box<ConditionalProbabilities>>,
  #[serde(flatten)]
  pub probabilities: HashMap<String, f64>,
}

impl ConditionalProbabilities {
  /// Check if this is a terminal node (has actual probabilities, not deeper nesting)
  pub fn is_terminal(&self) -> bool {
    self.deeper.is_none()
  }
}

/// A single node in the Bayesian network.
pub struct BayesianNode {
  definition: NodeDefinition,
}

impl BayesianNode {
  pub fn new(definition: NodeDefinition) -> Self {
    Self { definition }
  }

  pub fn name(&self) -> &str {
    &self.definition.name
  }

  pub fn parent_names(&self) -> &[String] {
    &self.definition.parent_names
  }

  pub fn possible_values(&self) -> &[String] {
    &self.definition.possible_values
  }

  /// Get the probability distribution given parent node values.
  fn get_probabilities_given_known_values(
    &self,
    parent_values: &HashMap<String, String>,
  ) -> HashMap<String, f64> {
    let mut probabilities = &self.definition.conditional_probabilities;

    for parent_name in &self.definition.parent_names {
      if let Some(deeper) = &probabilities.deeper {
        if let Some(parent_value) = parent_values.get(parent_name) {
          if let Some(next_level) = deeper.get(parent_value) {
            probabilities = next_level;
            continue;
          }
        }
        // Use skip if parent value not found in deeper
        if let Some(skip) = &probabilities.skip {
          probabilities = skip;
        }
      }
    }

    probabilities.probabilities.clone()
  }

  /// Randomly sample from the given values using the given probabilities.
  fn sample_random_value_from_possibilities(
    possible_values: &[String],
    total_probability: f64,
    probabilities: &HashMap<String, f64>,
  ) -> String {
    if possible_values.is_empty() {
      return String::new();
    }

    let mut rng = rand::rng();
    let anchor = rng.random::<f64>() * total_probability;
    let mut cumulative = 0.0;

    for value in possible_values {
      if let Some(&prob) = probabilities.get(value) {
        cumulative += prob;
        if cumulative > anchor {
          return value.clone();
        }
      }
    }

    possible_values.first().cloned().unwrap_or_default()
  }

  /// Sample a value from the conditional distribution given parent values.
  pub fn sample(&self, parent_values: &HashMap<String, String>) -> String {
    let probabilities = self.get_probabilities_given_known_values(parent_values);
    let possible_values: Vec<String> = probabilities.keys().cloned().collect();

    Self::sample_random_value_from_possibilities(&possible_values, 1.0, &probabilities)
  }

  /// Sample according to restrictions on possible values.
  ///
  /// Returns `None` if no valid value can be sampled.
  pub fn sample_according_to_restrictions(
    &self,
    parent_values: &HashMap<String, String>,
    value_possibilities: Option<&[String]>,
    banned_values: &[String],
  ) -> Option<String> {
    let probabilities = self.get_probabilities_given_known_values(parent_values);
    let values_in_distribution: Vec<String> = probabilities.keys().cloned().collect();

    let possible_values = value_possibilities.unwrap_or(&values_in_distribution);

    let mut valid_values = Vec::new();
    let mut total_probability = 0.0;

    for value in possible_values {
      if !banned_values.contains(value) && values_in_distribution.contains(value) {
        if let Some(&prob) = probabilities.get(value) {
          valid_values.push(value.clone());
          total_probability += prob;
        }
      }
    }

    if valid_values.is_empty() {
      return None;
    }

    Some(Self::sample_random_value_from_possibilities(
      &valid_values,
      total_probability,
      &probabilities,
    ))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn create_test_node() -> BayesianNode {
    let mut probs = HashMap::new();
    probs.insert("1920".to_string(), 0.5);
    probs.insert("1366".to_string(), 0.3);
    probs.insert("1536".to_string(), 0.2);

    let definition = NodeDefinition {
      name: "screen.width".to_string(),
      parent_names: vec![],
      possible_values: vec!["1920".to_string(), "1366".to_string(), "1536".to_string()],
      conditional_probabilities: ConditionalProbabilities {
        deeper: None,
        skip: None,
        probabilities: probs,
      },
    };

    BayesianNode::new(definition)
  }

  #[test]
  fn test_sample_returns_valid_value() {
    let node = create_test_node();
    let parent_values = HashMap::new();

    for _ in 0..100 {
      let value = node.sample(&parent_values);
      assert!(
        node.possible_values().contains(&value),
        "Sampled value '{}' not in possible values",
        value
      );
    }
  }

  #[test]
  fn test_sample_with_restrictions() {
    let node = create_test_node();
    let parent_values = HashMap::new();

    let allowed = vec!["1920".to_string()];
    let banned = vec![];

    let value = node.sample_according_to_restrictions(&parent_values, Some(&allowed), &banned);

    assert_eq!(value, Some("1920".to_string()));
  }

  #[test]
  fn test_sample_with_banned_values() {
    let node = create_test_node();
    let parent_values = HashMap::new();

    let banned = vec!["1920".to_string(), "1366".to_string()];

    for _ in 0..100 {
      let value = node.sample_according_to_restrictions(&parent_values, None, &banned);
      assert_eq!(value, Some("1536".to_string()));
    }
  }

  #[test]
  fn test_sample_returns_none_when_all_banned() {
    let node = create_test_node();
    let parent_values = HashMap::new();

    let banned = vec!["1920".to_string(), "1366".to_string(), "1536".to_string()];

    let value = node.sample_according_to_restrictions(&parent_values, None, &banned);
    assert!(value.is_none());
  }
}
