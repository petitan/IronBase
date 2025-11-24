// Label management for DOCJL blocks

use super::{DomainError, DomainResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Label structure: prefix:number (e.g., sec:4.2, tab:5, para:1)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Label {
    pub prefix: String,
    pub number: LabelNumber,
}

/// Label number - can be simple (5), hierarchical (4.2.1), or alphanumeric (test, demo)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LabelNumber {
    Simple(u32),
    Hierarchical(Vec<u32>),
    Alphanumeric(String),
}

impl Label {
    /// Parse a label string (e.g., "sec:4.2")
    pub fn parse(s: &str) -> DomainResult<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err(DomainError::InvalidLabel {
                label: s.to_string(),
                reason: "Must be in format 'prefix:number'".to_string(),
            });
        }

        let prefix = parts[0].to_string();
        let number_str = parts[1];

        let number = if number_str.contains('.') {
            let nums: Result<Vec<u32>, _> = number_str
                .split('.')
                .map(|n| n.parse::<u32>())
                .collect();
            match nums {
                Ok(nums) => LabelNumber::Hierarchical(nums),
                Err(_) => {
                    // Not a valid hierarchical number, treat as alphanumeric
                    LabelNumber::Alphanumeric(number_str.to_string())
                }
            }
        } else {
            match number_str.parse::<u32>() {
                Ok(n) => LabelNumber::Simple(n),
                Err(_) => {
                    // Not a number, accept as alphanumeric identifier
                    if number_str.is_empty() {
                        return Err(DomainError::InvalidLabel {
                            label: s.to_string(),
                            reason: "Label number cannot be empty".to_string(),
                        });
                    }
                    LabelNumber::Alphanumeric(number_str.to_string())
                }
            }
        };

        Ok(Label { prefix, number })
    }

    /// Convert label to string
    pub fn to_string(&self) -> String {
        match &self.number {
            LabelNumber::Simple(n) => format!("{}:{}", self.prefix, n),
            LabelNumber::Hierarchical(nums) => {
                let num_str = nums
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(".");
                format!("{}:{}", self.prefix, num_str)
            }
            LabelNumber::Alphanumeric(s) => format!("{}:{}", self.prefix, s),
        }
    }

    /// Increment the label number (for auto-generation)
    pub fn increment(&self) -> Self {
        let number = match &self.number {
            LabelNumber::Simple(n) => LabelNumber::Simple(n + 1),
            LabelNumber::Hierarchical(nums) => {
                let mut new_nums = nums.clone();
                if let Some(last) = new_nums.last_mut() {
                    *last += 1;
                }
                LabelNumber::Hierarchical(new_nums)
            }
            LabelNumber::Alphanumeric(s) => {
                // For alphanumeric labels, append "_1" or increment suffix
                if let Some((base, suffix)) = s.rsplit_once('_') {
                    if let Ok(num) = suffix.parse::<u32>() {
                        return Label {
                            prefix: self.prefix.clone(),
                            number: LabelNumber::Alphanumeric(format!("{}_{}", base, num + 1)),
                        };
                    }
                }
                // No suffix found, append "_1"
                LabelNumber::Alphanumeric(format!("{}_1", s))
            }
        };

        Label {
            prefix: self.prefix.clone(),
            number,
        }
    }

    /// Check if this label is a child of another (for hierarchical labels)
    pub fn is_child_of(&self, parent: &Label) -> bool {
        if self.prefix != parent.prefix {
            return false;
        }

        match (&self.number, &parent.number) {
            // Both hierarchical: check prefix match
            (LabelNumber::Hierarchical(child), LabelNumber::Hierarchical(parent)) => {
                if child.len() <= parent.len() {
                    return false;
                }
                child[..parent.len()] == parent[..]
            }
            // Child is hierarchical, parent is simple: check if child starts with parent number
            (LabelNumber::Hierarchical(child), LabelNumber::Simple(parent_num)) => {
                if child.is_empty() {
                    return false;
                }
                child[0] == *parent_num
            }
            // Both simple or parent hierarchical: not a child relationship
            _ => false,
        }
    }
}

/// Label generator - auto-generates labels for new blocks
pub struct LabelGenerator {
    /// Tracks the highest number for each prefix
    counters: HashMap<String, u32>,
    /// All existing labels (for uniqueness check)
    existing: HashSet<String>,
}

impl LabelGenerator {
    /// Create a new label generator
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            existing: HashSet::new(),
        }
    }

    /// Register an existing label
    pub fn register(&mut self, label: &str) -> DomainResult<()> {
        if self.existing.contains(label) {
            return Err(DomainError::DuplicateLabel {
                label: label.to_string(),
            });
        }

        let parsed = Label::parse(label)?;
        self.existing.insert(label.to_string());

        // Update counter if this number is higher (only for numeric labels)
        let num = match parsed.number {
            LabelNumber::Simple(n) => n,
            LabelNumber::Hierarchical(nums) => *nums.first().unwrap_or(&0),
            LabelNumber::Alphanumeric(_) => return Ok(()), // Alphanumeric labels don't update counters
        };

        self.counters
            .entry(parsed.prefix.clone())
            .and_modify(|counter| {
                if num > *counter {
                    *counter = num;
                }
            })
            .or_insert(num);

        Ok(())
    }

    /// Generate a new label with the given prefix
    pub fn generate(&mut self, prefix: &str) -> String {
        let counter = self.counters.entry(prefix.to_string()).or_insert(0);
        *counter += 1;

        let label = format!("{}:{}", prefix, counter);

        // Ensure uniqueness (shouldn't happen, but defensive)
        let mut attempt = 0;
        let mut final_label = label.clone();
        while self.existing.contains(&final_label) {
            attempt += 1;
            final_label = format!("{}:{}_{}", prefix, counter, attempt);
        }

        self.existing.insert(final_label.clone());
        final_label
    }

    /// Get the next label without incrementing counter
    pub fn peek(&self, prefix: &str) -> String {
        let next_num = self.counters.get(prefix).map(|n| n + 1).unwrap_or(1);
        format!("{}:{}", prefix, next_num)
    }

    /// Check if a label exists
    pub fn exists(&self, label: &str) -> bool {
        self.existing.contains(label)
    }

    /// Remove a label (when block is deleted)
    pub fn remove(&mut self, label: &str) {
        self.existing.remove(label);
    }
}

impl Default for LabelGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Label renumberer - renumbers labels when blocks are moved
pub struct LabelRenumberer {
    /// Mapping of old labels to new labels
    changes: HashMap<String, String>,
}

impl LabelRenumberer {
    pub fn new() -> Self {
        Self {
            changes: HashMap::new(),
        }
    }

    /// Record a label change
    pub fn record_change(&mut self, old_label: String, new_label: String) {
        self.changes.insert(old_label, new_label);
    }

    /// Get the new label for an old label (or return the original if not changed)
    pub fn resolve(&self, label: &str) -> String {
        self.changes
            .get(label)
            .cloned()
            .unwrap_or_else(|| label.to_string())
    }

    /// Get all changes
    pub fn get_changes(&self) -> Vec<(String, String)> {
        self.changes
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Apply renumbering to a section (e.g., when moving sec:4 to sec:5)
    pub fn renumber_section(
        &mut self,
        old_parent: &Label,
        new_parent: &Label,
        labels: &[String],
    ) -> DomainResult<()> {
        for label_str in labels {
            let label = Label::parse(label_str)?;
            if label.is_child_of(old_parent) {
                // Replace the parent part with the new parent
                let old_str = old_parent.to_string();
                let new_str = new_parent.to_string();
                let new_label_str = label_str.replace(&old_str, &new_str);
                self.record_change(label_str.clone(), new_label_str);
            }
        }
        Ok(())
    }
}

impl Default for LabelRenumberer {
    fn default() -> Self {
        Self::new()
    }
}

/// Get default prefix for a block type
pub fn default_prefix_for_type(block_type: &str) -> &'static str {
    match block_type {
        "paragraph" => "para",
        "heading" => "sec",
        "table" => "tab",
        "list" => "list",
        "image" => "fig",
        "code" => "code",
        "section" => "sec",
        _ => "block",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_parse() {
        let label = Label::parse("sec:4").unwrap();
        assert_eq!(label.prefix, "sec");
        assert_eq!(label.number, LabelNumber::Simple(4));
        assert_eq!(label.to_string(), "sec:4");

        let label = Label::parse("sec:4.2.1").unwrap();
        assert_eq!(label.prefix, "sec");
        assert_eq!(label.number, LabelNumber::Hierarchical(vec![4, 2, 1]));
        assert_eq!(label.to_string(), "sec:4.2.1");
    }

    #[test]
    fn test_label_increment() {
        let label = Label::parse("sec:4").unwrap();
        let next = label.increment();
        assert_eq!(next.to_string(), "sec:5");

        let label = Label::parse("sec:4.2").unwrap();
        let next = label.increment();
        assert_eq!(next.to_string(), "sec:4.3");
    }

    #[test]
    fn test_label_is_child_of() {
        let parent = Label::parse("sec:4").unwrap();
        let child = Label::parse("sec:4.1").unwrap();
        let not_child = Label::parse("sec:5").unwrap();

        assert!(child.is_child_of(&parent));
        assert!(!not_child.is_child_of(&parent));
    }

    #[test]
    fn test_label_generator() {
        let mut gen = LabelGenerator::new();

        gen.register("para:1").unwrap();
        gen.register("para:2").unwrap();

        let next = gen.generate("para");
        assert_eq!(next, "para:3");

        let next = gen.generate("sec");
        assert_eq!(next, "sec:1");

        assert!(gen.exists("para:1"));
        assert!(gen.exists("para:3"));
        assert!(!gen.exists("para:99"));
    }

    #[test]
    fn test_label_generator_duplicate() {
        let mut gen = LabelGenerator::new();
        gen.register("para:1").unwrap();
        let result = gen.register("para:1");
        assert!(result.is_err());
    }

    #[test]
    fn test_label_renumberer() {
        let mut renumberer = LabelRenumberer::new();
        renumberer.record_change("sec:4".to_string(), "sec:5".to_string());

        assert_eq!(renumberer.resolve("sec:4"), "sec:5");
        assert_eq!(renumberer.resolve("sec:3"), "sec:3");
    }

    #[test]
    fn test_renumber_section() {
        let mut renumberer = LabelRenumberer::new();
        let old_parent = Label::parse("sec:4").unwrap();
        let new_parent = Label::parse("sec:5").unwrap();

        let labels = vec![
            "sec:4.1".to_string(),
            "sec:4.2".to_string(),
            "sec:3.1".to_string(), // Not a child
        ];

        renumberer
            .renumber_section(&old_parent, &new_parent, &labels)
            .unwrap();

        assert_eq!(renumberer.resolve("sec:4.1"), "sec:5.1");
        assert_eq!(renumberer.resolve("sec:4.2"), "sec:5.2");
        assert_eq!(renumberer.resolve("sec:3.1"), "sec:3.1");
    }
}
