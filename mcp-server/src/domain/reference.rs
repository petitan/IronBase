// Cross-reference management for DOCJL documents

use super::{Block, DomainError, DomainResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Cross-reference tracker
pub struct CrossReference {
    /// Map: source label → set of target labels it references
    references: HashMap<String, HashSet<String>>,

    /// Reverse map: target label → set of source labels that reference it
    referenced_by: HashMap<String, HashSet<String>>,

    /// All valid labels in the document
    valid_labels: HashSet<String>,
}

impl CrossReference {
    pub fn new() -> Self {
        Self {
            references: HashMap::new(),
            referenced_by: HashMap::new(),
            valid_labels: HashSet::new(),
        }
    }

    /// Register a label as valid
    pub fn register_label(&mut self, label: String) {
        self.valid_labels.insert(label);
    }

    /// Remove a label (when block is deleted)
    pub fn remove_label(&mut self, label: &str) {
        self.valid_labels.remove(label);

        // Clean up any references from this label
        if let Some(targets) = self.references.remove(label) {
            for target in targets {
                if let Some(refs) = self.referenced_by.get_mut(&target) {
                    refs.remove(label);
                }
            }
        }

        // Clean up any references to this label
        if let Some(sources) = self.referenced_by.remove(label) {
            for source in sources {
                if let Some(refs) = self.references.get_mut(&source) {
                    refs.remove(label);
                }
            }
        }
    }

    /// Add a reference from source to target
    pub fn add_reference(&mut self, source: String, target: String) {
        self.references
            .entry(source.clone())
            .or_insert_with(HashSet::new)
            .insert(target.clone());

        self.referenced_by
            .entry(target)
            .or_insert_with(HashSet::new)
            .insert(source);
    }

    /// Remove a specific reference
    pub fn remove_reference(&mut self, source: &str, target: &str) {
        if let Some(refs) = self.references.get_mut(source) {
            refs.remove(target);
        }
        if let Some(refs) = self.referenced_by.get_mut(target) {
            refs.remove(source);
        }
    }

    /// Get all targets referenced by a source
    pub fn get_references(&self, source: &str) -> Vec<String> {
        self.references
            .get(source)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all sources that reference a target
    pub fn get_referenced_by(&self, target: &str) -> Vec<String> {
        self.referenced_by
            .get(target)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Update all references when a label changes
    pub fn update_label(&mut self, old_label: &str, new_label: String) {
        // Update as a valid label
        self.valid_labels.remove(old_label);
        self.valid_labels.insert(new_label.clone());

        // Update references FROM this label
        if let Some(targets) = self.references.remove(old_label) {
            for target in &targets {
                if let Some(refs) = self.referenced_by.get_mut(target) {
                    refs.remove(old_label);
                    refs.insert(new_label.clone());
                }
            }
            self.references.insert(new_label.clone(), targets);
        }

        // Update references TO this label
        if let Some(sources) = self.referenced_by.remove(old_label) {
            for source in &sources {
                if let Some(refs) = self.references.get_mut(source) {
                    refs.remove(old_label);
                    refs.insert(new_label.clone());
                }
            }
            self.referenced_by.insert(new_label, sources);
        }
    }

    /// Find broken references (references to non-existent labels)
    pub fn find_broken_references(&self) -> Vec<BrokenReference> {
        let mut broken = Vec::new();

        for (source, targets) in &self.references {
            for target in targets {
                if !self.valid_labels.contains(target) {
                    broken.push(BrokenReference {
                        source: source.clone(),
                        target: target.clone(),
                        error: format!("Target label '{}' does not exist", target),
                    });
                }
            }
        }

        broken
    }

    /// Check if a label can be safely deleted
    pub fn can_delete(&self, label: &str) -> DomainResult<()> {
        if let Some(sources) = self.referenced_by.get(label) {
            if !sources.is_empty() {
                return Err(DomainError::BrokenReference {
                    source: sources.iter().next().unwrap().clone(),
                    target: label.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Get all labels that would be affected by deleting a label
    pub fn get_affected_by_deletion(&self, label: &str) -> Vec<String> {
        self.get_referenced_by(label)
    }

    /// Extract references from a block and register them
    pub fn extract_and_register(&mut self, block: &Block) {
        if let Some(source_label) = block.label() {
            let refs = block.extract_references();
            for target in refs {
                self.add_reference(source_label.to_string(), target);
            }
        }

        // Recursively process children
        if let Some(children) = block.children() {
            for child in children {
                self.extract_and_register(child);
            }
        }
    }
}

impl Default for CrossReference {
    fn default() -> Self {
        Self::new()
    }
}

/// Broken reference information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokenReference {
    pub source: String,
    pub target: String,
    pub error: String,
}

/// Reference validator - validates all cross-references in a document
pub struct ReferenceValidator {
    cross_ref: CrossReference,
}

impl ReferenceValidator {
    pub fn new() -> Self {
        Self {
            cross_ref: CrossReference::new(),
        }
    }

    /// Build reference map from document blocks
    pub fn build_from_blocks(&mut self, blocks: &[Block]) {
        self.register_labels(blocks);
        self.extract_references(blocks);
    }

    /// Register all labels in blocks
    fn register_labels(&mut self, blocks: &[Block]) {
        for block in blocks {
            if let Some(label) = block.label() {
                self.cross_ref.register_label(label.to_string());
            }

            if let Some(children) = block.children() {
                self.register_labels(children);
            }
        }
    }

    /// Extract all references from blocks
    fn extract_references(&mut self, blocks: &[Block]) {
        for block in blocks {
            self.cross_ref.extract_and_register(block);
        }
    }

    /// Validate all references
    pub fn validate(&self) -> Vec<BrokenReference> {
        self.cross_ref.find_broken_references()
    }

    /// Check if deleting a label would break references
    pub fn check_deletion(&self, label: &str) -> DomainResult<Vec<String>> {
        let affected = self.cross_ref.get_affected_by_deletion(label);
        if affected.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(affected)
        }
    }

    /// Update reference when label changes
    pub fn update_label(&mut self, old_label: &str, new_label: String) {
        self.cross_ref.update_label(old_label, new_label);
    }

    /// Get the cross-reference tracker
    pub fn cross_ref(&self) -> &CrossReference {
        &self.cross_ref
    }

    /// Get mutable cross-reference tracker
    pub fn cross_ref_mut(&mut self) -> &mut CrossReference {
        &mut self.cross_ref
    }
}

impl Default for ReferenceValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::block::{InlineContent, Paragraph};

    #[test]
    fn test_cross_reference_basic() {
        let mut cr = CrossReference::new();

        cr.register_label("sec:1".to_string());
        cr.register_label("sec:2".to_string());

        cr.add_reference("sec:1".to_string(), "sec:2".to_string());

        assert_eq!(cr.get_references("sec:1"), vec!["sec:2"]);
        assert_eq!(cr.get_referenced_by("sec:2"), vec!["sec:1"]);
    }

    #[test]
    fn test_find_broken_references() {
        let mut cr = CrossReference::new();

        cr.register_label("sec:1".to_string());
        cr.add_reference("sec:1".to_string(), "sec:99".to_string());

        let broken = cr.find_broken_references();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].source, "sec:1");
        assert_eq!(broken[0].target, "sec:99");
    }

    #[test]
    fn test_update_label() {
        let mut cr = CrossReference::new();

        cr.register_label("sec:1".to_string());
        cr.register_label("sec:2".to_string());
        cr.add_reference("sec:1".to_string(), "sec:2".to_string());

        // Rename sec:2 to sec:3
        cr.update_label("sec:2", "sec:3".to_string());

        assert_eq!(cr.get_references("sec:1"), vec!["sec:3"]);
        assert_eq!(cr.get_referenced_by("sec:3"), vec!["sec:1"]);
        assert!(cr.get_referenced_by("sec:2").is_empty());
    }

    #[test]
    fn test_can_delete() {
        let mut cr = CrossReference::new();

        cr.register_label("sec:1".to_string());
        cr.register_label("sec:2".to_string());
        cr.add_reference("sec:1".to_string(), "sec:2".to_string());

        // Can't delete sec:2 because sec:1 references it
        assert!(cr.can_delete("sec:2").is_err());

        // Can delete sec:1 (nothing references it)
        assert!(cr.can_delete("sec:1").is_ok());
    }

    #[test]
    fn test_extract_and_register() {
        let mut cr = CrossReference::new();

        let block = Block::Paragraph(Paragraph {
            label: Some("para:1".to_string()),
            content: vec![
                InlineContent::Text {
                    content: "See ".to_string(),
                },
                InlineContent::Ref {
                    target: "sec:4".to_string(),
                },
            ],
            compliance_note: None,
        });

        cr.register_label("para:1".to_string());
        cr.register_label("sec:4".to_string());
        cr.extract_and_register(&block);

        assert_eq!(cr.get_references("para:1"), vec!["sec:4"]);
        assert_eq!(cr.get_referenced_by("sec:4"), vec!["para:1"]);
    }

    #[test]
    fn test_remove_label() {
        let mut cr = CrossReference::new();

        cr.register_label("sec:1".to_string());
        cr.register_label("sec:2".to_string());
        cr.add_reference("sec:1".to_string(), "sec:2".to_string());

        cr.remove_label("sec:1");

        assert!(cr.get_references("sec:1").is_empty());
        assert!(cr.get_referenced_by("sec:2").is_empty());
    }
}
