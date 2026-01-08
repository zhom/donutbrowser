//! Bayesian network for fingerprint generation.
//!
//! Loads pre-trained probability distributions from ZIP files and samples fingerprints.

use super::bayesian_node::{BayesianNode, NodeDefinition};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use zip::ZipArchive;

/// Network definition structure from the ZIP file.
#[derive(Debug, Deserialize)]
pub struct NetworkDefinition {
  pub nodes: Vec<NodeDefinition>,
}

/// A Bayesian network for generating consistent fingerprints.
pub struct BayesianNetwork {
  nodes_in_sampling_order: Vec<BayesianNode>,
  nodes_by_name: HashMap<String, usize>,
}

impl BayesianNetwork {
  /// Load a Bayesian network from embedded ZIP file bytes.
  pub fn from_zip_bytes(zip_bytes: &[u8]) -> Result<Self, BayesianNetworkError> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor)?;

    // Find and read the JSON file from the ZIP
    let mut json_content = String::new();
    for i in 0..archive.len() {
      let mut file = archive.by_index(i)?;
      if file.name().ends_with(".json") {
        file.read_to_string(&mut json_content)?;
        break;
      }
    }

    if json_content.is_empty() {
      return Err(BayesianNetworkError::NoJsonInZip);
    }

    let definition: NetworkDefinition = serde_json::from_str(&json_content)?;

    let mut nodes_in_sampling_order = Vec::with_capacity(definition.nodes.len());
    let mut nodes_by_name = HashMap::with_capacity(definition.nodes.len());

    for (i, node_def) in definition.nodes.into_iter().enumerate() {
      nodes_by_name.insert(node_def.name.clone(), i);
      nodes_in_sampling_order.push(BayesianNode::new(node_def));
    }

    Ok(Self {
      nodes_in_sampling_order,
      nodes_by_name,
    })
  }

  /// Get a node by name.
  pub fn get_node(&self, name: &str) -> Option<&BayesianNode> {
    self
      .nodes_by_name
      .get(name)
      .map(|&i| &self.nodes_in_sampling_order[i])
  }

  /// Get possible values for a node.
  pub fn get_possible_values(&self, name: &str) -> Option<Vec<String>> {
    self
      .get_node(name)
      .map(|node| node.possible_values().to_vec())
  }

  /// Generate a random sample from the network.
  ///
  /// `input_values` contains already known node values that should not be overwritten.
  pub fn generate_sample(&self, input_values: &HashMap<String, String>) -> HashMap<String, String> {
    let mut sample = input_values.clone();

    for node in &self.nodes_in_sampling_order {
      if !sample.contains_key(node.name()) {
        let value = node.sample(&sample);
        sample.insert(node.name().to_string(), value);
      }
    }

    sample
  }

  /// Generate a random sample consistent with the given value restrictions.
  ///
  /// Uses backtracking to find a valid configuration.
  /// Returns `None` if no consistent sample can be generated.
  pub fn generate_consistent_sample_when_possible(
    &self,
    value_possibilities: &HashMap<String, Vec<String>>,
  ) -> Option<HashMap<String, String>> {
    self.recursively_generate_consistent_sample(HashMap::new(), value_possibilities, 0)
  }

  fn recursively_generate_consistent_sample(
    &self,
    sample_so_far: HashMap<String, String>,
    value_possibilities: &HashMap<String, Vec<String>>,
    depth: usize,
  ) -> Option<HashMap<String, String>> {
    if depth >= self.nodes_in_sampling_order.len() {
      return Some(sample_so_far);
    }

    let node = &self.nodes_in_sampling_order[depth];
    let mut banned_values: Vec<String> = Vec::new();
    let mut sample_so_far = sample_so_far;

    loop {
      let sample_value = node.sample_according_to_restrictions(
        &sample_so_far,
        value_possibilities.get(node.name()).map(|v| v.as_slice()),
        &banned_values,
      );

      let Some(value) = sample_value else {
        break;
      };

      sample_so_far.insert(node.name().to_string(), value.clone());

      if let Some(complete_sample) = self.recursively_generate_consistent_sample(
        sample_so_far.clone(),
        value_possibilities,
        depth + 1,
      ) {
        return Some(complete_sample);
      }

      banned_values.push(value);
    }

    None
  }
}

/// Errors that can occur when working with Bayesian networks.
#[derive(Debug, thiserror::Error)]
pub enum BayesianNetworkError {
  #[error("ZIP file error: {0}")]
  Zip(#[from] zip::result::ZipError),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("JSON parsing error: {0}")]
  Json(#[from] serde_json::Error),

  #[error("No JSON file found in ZIP archive")]
  NoJsonInZip,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_load_input_network() {
    let zip_bytes = include_bytes!("../data/input-network-definition.zip");
    let network = BayesianNetwork::from_zip_bytes(zip_bytes);
    assert!(
      network.is_ok(),
      "Failed to load input network: {:?}",
      network.err()
    );
  }

  #[test]
  fn test_generate_sample_from_input_network() {
    let zip_bytes = include_bytes!("../data/input-network-definition.zip");
    let network = BayesianNetwork::from_zip_bytes(zip_bytes).unwrap();

    let sample = network.generate_sample(&HashMap::new());
    assert!(!sample.is_empty(), "Sample should not be empty");
  }

  #[test]
  fn test_generate_consistent_sample() {
    let zip_bytes = include_bytes!("../data/input-network-definition.zip");
    let network = BayesianNetwork::from_zip_bytes(zip_bytes).unwrap();

    let mut constraints = HashMap::new();
    constraints.insert("*OPERATING_SYSTEM".to_string(), vec!["windows".to_string()]);

    let sample = network.generate_consistent_sample_when_possible(&constraints);
    assert!(sample.is_some(), "Should generate a consistent sample");

    if let Some(s) = sample {
      assert_eq!(s.get("*OPERATING_SYSTEM"), Some(&"windows".to_string()));
    }
  }
}
