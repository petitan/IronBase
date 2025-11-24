// Schema validation for DOCJL documents

use super::{Block, Document};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result of a validation operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationResult {
    pub fn success() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn with_error(error: ValidationError) -> Self {
        Self {
            valid: false,
            errors: vec![error],
            warnings: Vec::new(),
        }
    }

    pub fn add_error(&mut self, error: ValidationError) {
        self.errors.push(error);
        self.valid = false;
    }

    pub fn add_warning(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
    }

    pub fn merge(&mut self, other: ValidationResult) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        self.valid = self.valid && other.valid;
    }
}

/// Validation error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub block_label: Option<String>,
    pub field: Option<String>,
    pub message: String,
    pub error_type: ErrorType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorType {
    MissingField,
    InvalidType,
    InvalidValue,
    SchemaViolation,
    ReferenceError,
}

/// Validation warning (non-fatal)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub block_label: Option<String>,
    pub message: String,
}

/// Schema validator
pub struct SchemaValidator {
    schema: Option<Value>,
    strict_mode: bool,
}

impl SchemaValidator {
    pub fn new(schema: Option<Value>, strict_mode: bool) -> Self {
        Self {
            schema,
            strict_mode,
        }
    }

    /// Validate a complete document
    pub fn validate_document(&self, document: &Document) -> ValidationResult {
        let mut result = ValidationResult::success();

        // Validate metadata
        if document.metadata.title.is_empty() {
            result.add_error(ValidationError {
                block_label: None,
                field: Some("title".to_string()),
                message: "Document title is required".to_string(),
                error_type: ErrorType::MissingField,
            });
        }

        if document.metadata.version.is_empty() {
            result.add_error(ValidationError {
                block_label: None,
                field: Some("version".to_string()),
                message: "Document version is required".to_string(),
                error_type: ErrorType::MissingField,
            });
        }

        // Validate blocks
        for block in &document.docjll {
            let block_result = self.validate_block(block);
            result.merge(block_result);
        }

        result
    }

    /// Validate a single block
    pub fn validate_block(&self, block: &Block) -> ValidationResult {
        let mut result = ValidationResult::success();

        // Check label format if present
        if let Some(label) = block.label() {
            if let Err(e) = crate::domain::label::Label::parse(label) {
                result.add_error(ValidationError {
                    block_label: Some(label.to_string()),
                    field: Some("label".to_string()),
                    message: format!("Invalid label format: {}", e),
                    error_type: ErrorType::InvalidValue,
                });
            }
        }

        // Block-specific validation
        match block {
            Block::Paragraph(p) => {
                if p.content.is_empty() && self.strict_mode {
                    result.add_warning(ValidationWarning {
                        block_label: p.label.clone(),
                        message: "Paragraph has no content".to_string(),
                    });
                }
            }
            Block::Heading(h) => {
                // Only validate level if it's specified
                if let Some(level) = h.level {
                    if level < 1 || level > 6 {
                        result.add_error(ValidationError {
                            block_label: h.label.clone(),
                            field: Some("level".to_string()),
                            message: format!("Heading level {} is invalid (must be 1-6)", level),
                            error_type: ErrorType::InvalidValue,
                        });
                    }
                }
                if h.content.is_empty() {
                    result.add_error(ValidationError {
                        block_label: h.label.clone(),
                        field: Some("content".to_string()),
                        message: "Heading content is required".to_string(),
                        error_type: ErrorType::MissingField,
                    });
                }

                // Validate children
                if let Some(children) = &h.children {
                    for child in children {
                        result.merge(self.validate_block(child));
                    }
                }
            }
            Block::Table(t) => {
                if t.headers.is_empty() {
                    result.add_error(ValidationError {
                        block_label: t.label.clone(),
                        field: Some("headers".to_string()),
                        message: "Table must have headers".to_string(),
                        error_type: ErrorType::MissingField,
                    });
                }
                if t.rows.is_empty() && self.strict_mode {
                    result.add_warning(ValidationWarning {
                        block_label: t.label.clone(),
                        message: "Table has no rows".to_string(),
                    });
                }

                // Check row column counts match headers
                for (i, row) in t.rows.iter().enumerate() {
                    if row.len() != t.headers.len() {
                        result.add_error(ValidationError {
                            block_label: t.label.clone(),
                            field: Some(format!("rows[{}]", i)),
                            message: format!(
                                "Row {} has {} columns but headers have {}",
                                i,
                                row.len(),
                                t.headers.len()
                            ),
                            error_type: ErrorType::InvalidValue,
                        });
                    }
                }

                if t.caption.is_none() && self.strict_mode {
                    result.add_warning(ValidationWarning {
                        block_label: t.label.clone(),
                        message: "Table caption is recommended".to_string(),
                    });
                }
            }
            Block::List(l) => {
                if l.items.is_empty() && self.strict_mode {
                    result.add_warning(ValidationWarning {
                        block_label: l.label.clone(),
                        message: "List has no items".to_string(),
                    });
                }
            }
            Block::Section(s) => {
                if s.title.is_empty() {
                    result.add_error(ValidationError {
                        block_label: s.label.clone(),
                        field: Some("title".to_string()),
                        message: "Section title is required".to_string(),
                        error_type: ErrorType::MissingField,
                    });
                }

                // Validate children
                for child in &s.children {
                    result.merge(self.validate_block(child));
                }
            }
            Block::Image(i) => {
                if i.src.is_empty() {
                    result.add_error(ValidationError {
                        block_label: i.label.clone(),
                        field: Some("src".to_string()),
                        message: "Image source is required".to_string(),
                        error_type: ErrorType::MissingField,
                    });
                }
                if i.alt.is_none() && self.strict_mode {
                    result.add_warning(ValidationWarning {
                        block_label: i.label.clone(),
                        message: "Image alt text is recommended for accessibility".to_string(),
                    });
                }
            }
            Block::CodeBlock(c) => {
                if c.content.is_empty() {
                    result.add_error(ValidationError {
                        block_label: c.label.clone(),
                        field: Some("content".to_string()),
                        message: "Code block content is required".to_string(),
                        error_type: ErrorType::MissingField,
                    });
                }
            }
            // For now, skip validation for other new block types
            _ => {}
        }

        // Apply JSON schema validation if provided
        if let Some(schema) = &self.schema {
            let schema_result = self.validate_against_schema(block, schema);
            result.merge(schema_result);
        }

        result
    }

    /// Validate block against JSON schema
    fn validate_against_schema(&self, _block: &Block, _schema: &Value) -> ValidationResult {
        // TODO: Implement JSON schema validation using jsonschema crate
        // For now, return success
        ValidationResult::success()
    }

    /// Quick check if a block is valid
    pub fn is_valid_block(&self, block: &Block) -> bool {
        self.validate_block(block).valid
    }
}

impl Default for SchemaValidator {
    fn default() -> Self {
        Self::new(None, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::block::{Heading, InlineContent, Paragraph, Table};

    #[test]
    fn test_validate_paragraph() {
        let validator = SchemaValidator::default();

        let block = Block::Paragraph(Paragraph {
            content: vec![InlineContent::Text {
                content: "Test".to_string(),
            }],
            label: Some("para:1".to_string()),
            compliance_note: None,
        });

        let result = validator.validate_block(&block);
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_heading_invalid_level() {
        let validator = SchemaValidator::default();

        let block = Block::Heading(Heading {
            level: Some(10),
            content: vec![InlineContent::Text {
                content: "Test".to_string(),
            }],
            label: Some("sec:1".to_string()),
            children: None,
        });

        let result = validator.validate_block(&block);
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].error_type, ErrorType::InvalidValue);
    }

    #[test]
    fn test_validate_table_column_mismatch() {
        let validator = SchemaValidator::default();

        let block = Block::Table(Table {
            headers: vec!["Col1".to_string(), "Col2".to_string()],
            rows: vec![
                vec!["A".to_string(), "B".to_string()],      // OK
                vec!["C".to_string()],                        // Wrong column count
            ],
            caption: Some("Test Table".to_string()),
            label: Some("tab:1".to_string()),
        });

        let result = validator.validate_block(&block);
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_validate_strict_mode_warnings() {
        let validator = SchemaValidator::new(None, true);

        let block = Block::Paragraph(Paragraph {
            content: vec![],
            label: Some("para:1".to_string()),
            compliance_note: None,
        });

        let result = validator.validate_block(&block);
        assert!(result.valid); // Still valid, just warnings
        assert_eq!(result.warnings.len(), 1);
    }
}
