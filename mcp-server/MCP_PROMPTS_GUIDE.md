# MCP Prompts Guide

## Overview

This guide documents all **15 MCP prompt templates** available in the DOCJL Editor server. Prompts help Claude Desktop users perform common document operations efficiently.

## Prompt Categories

- **Balanced MVP** (10 prompts): General document operations
- **ISO 17025 Calibration** (5 prompts): Specialized calibration/metrology prompts

---

## Balanced MVP Prompts (10)

### 1. validate-structure

**Purpose**: Validate DOCJL document structure and label hierarchy.

**Arguments**:
- `document_id` (required): Document ID to validate

**Use Cases**:
- Check label format compliance (type:id pattern)
- Verify hierarchical consistency (sec:1 → sec:1.1 → sec:1.1.1)
- Detect duplicate labels
- Ensure all required fields present

**Example Usage**:
```
Use the validate-structure prompt on document "quality_manual_v2" to check for structural issues.
```

**What Claude Will Do**:
- Retrieve document via `mcp_docjl_get_document`
- Check all labels follow pattern: `^(para|sec|fig|tbl|eq|lst|def|thm|lem|proof|ex|note|warn|info|tip):(.+)$`
- Verify hierarchical labels (sec:4.2.1 must have parent sec:4.2)
- Report any violations or confirm validity

---

### 2. validate-compliance

**Purpose**: Check ISO 17025 compliance requirements.

**Arguments**:
- `document_id` (required): Document ID (Quality Manual, SOP, etc.)
- `document_type` (required): Type of document
  - `quality_manual` - Quality management system manual
  - `sop` - Standard Operating Procedure
  - `work_instruction` - Detailed work instruction
  - `audit_report` - Internal/external audit report

**Use Cases**:
- Verify ISO 17025:2017 clause coverage
- Check mandatory sections present
- Ensure traceability requirements met
- Validate calibration documentation

**Example Usage**:
```
Run validate-compliance on "sop_calibration_thermometer" as document_type "sop" to check ISO 17025 requirements.
```

**What Claude Will Do**:
- Load document and identify document type
- Cross-reference with ISO 17025:2017 requirements
- Check for mandatory clauses (4.1-8.9 depending on type)
- Report missing requirements or gaps

---

### 3. create-section

**Purpose**: Create new DOCJL section with appropriate structure.

**Arguments**:
- `section_type` (required): Type of section to create
  - `procedure` - Step-by-step procedure
  - `requirement` - Requirement or specification
  - `evidence` - Evidence or record reference
  - `reference` - External reference or citation
- `topic` (required): Section topic/title

**Use Cases**:
- Add new procedure to SOP
- Document new requirement
- Create evidence section for audit
- Add reference section

**Example Usage**:
```
Create a new "procedure" section about "Temperature Calibration Process" in the current document.
```

**What Claude Will Do**:
- Generate appropriate DOCJL structure for section_type
- Include proper label (auto-incremented or suggested)
- Add placeholder content based on type
- Return formatted section ready to insert via `mcp_docjl_insert_block`

---

### 4. summarize-document

**Purpose**: Generate executive summary of DOCJL document.

**Arguments**:
- `document_id` (required): Document ID to summarize
- `max_length` (optional): Maximum summary length in words (default: 250)

**Use Cases**:
- Create abstract for technical document
- Generate executive summary for reports
- Quick overview of SOPs
- Audit report summary

**Example Usage**:
```
Summarize document "mk_manual_complete" in max 150 words.
```

**What Claude Will Do**:
- Retrieve full document
- Extract key points from all top-level sections
- Identify main purpose, scope, and key findings
- Generate concise summary respecting word limit

---

### 5. suggest-improvements

**Purpose**: Analyze document and suggest improvements.

**Arguments**:
- `document_id` (required): Document ID to analyze
- `focus_areas` (optional): Areas to focus on (comma-separated)
  - `clarity` - Writing clarity and readability
  - `completeness` - Missing information or sections
  - `compliance` - ISO 17025 compliance gaps
  - `consistency` - Internal consistency issues

**Use Cases**:
- Peer review assistance
- Pre-audit document review
- Quality improvement
- Writing style enhancement

**Example Usage**:
```
Analyze "sop_measurement_uncertainty" focusing on "clarity,completeness" and suggest improvements.
```

**What Claude Will Do**:
- Read entire document
- Check specified focus areas
- Identify specific issues with examples
- Provide actionable improvement suggestions

---

### 6. audit-readiness

**Purpose**: Check document readiness for ISO 17025 audit.

**Arguments**:
- `document_id` (required): Document ID to check
- `audit_scope` (optional): Audit scope
  - `full` - Complete ISO 17025 audit (default)
  - `partial` - Partial/surveillance audit
  - `specific_clause` - Specific clause (e.g., "5.6 Measurement Traceability")

**Use Cases**:
- Pre-audit document review
- Internal audit preparation
- Surveillance audit readiness
- Accreditation renewal prep

**Example Usage**:
```
Check audit-readiness of "quality_manual_2024" for a "full" ISO 17025 audit.
```

**What Claude Will Do**:
- Review document against audit scope
- Check all required clauses covered
- Verify evidence and records referenced
- Generate readiness checklist with gaps

---

### 7. create-outline

**Purpose**: Generate document outline based on document type.

**Arguments**:
- `document_type` (required): Type of document
  - `quality_manual` - QMS manual structure
  - `sop` - Standard Operating Procedure
  - `work_instruction` - Work instruction template
- `topic` (required): Document topic

**Use Cases**:
- Start new document from template
- Restructure existing document
- Ensure standard format compliance
- Training material creation

**Example Usage**:
```
Create an outline for a "sop" about "Pipette Calibration Procedure".
```

**What Claude Will Do**:
- Generate appropriate structure for document_type
- Include standard sections (Purpose, Scope, Procedure, etc.)
- Add placeholder labels (sec:1, sec:2, etc.)
- Return ready-to-use DOCJL structure

---

### 8. analyze-changes

**Purpose**: Compare two document versions and analyze changes.

**Arguments**:
- `document_id_old` (required): Old version document ID
- `document_id_new` (required): New version document ID

**Use Cases**:
- Version control review
- Change impact analysis
- Audit trail documentation
- Training on updates

**Example Usage**:
```
Compare changes between "sop_v1" and "sop_v2" and analyze impact.
```

**What Claude Will Do**:
- Retrieve both documents
- Identify added/removed/modified sections
- Highlight significant changes
- Assess impact on compliance or procedures

---

### 9. check-consistency

**Purpose**: Check internal consistency across document sections.

**Arguments**:
- `document_id` (required): Document ID to check

**Use Cases**:
- Quality assurance
- Pre-publication review
- Terminology consistency
- Cross-reference validation

**Example Usage**:
```
Run check-consistency on "quality_manual" to find internal inconsistencies.
```

**What Claude Will Do**:
- Scan entire document
- Check terminology usage consistency
- Verify cross-references valid
- Identify contradictions or conflicts
- Report inconsistencies with locations

---

### 10. resolve-reference

**Purpose**: Resolve and explain a label reference.

**Arguments**:
- `document_id` (required): Document ID
- `label` (required): Label to resolve (e.g., "sec:4.2.1")
- `include_context` (optional): Include surrounding context (default: true)

**Use Cases**:
- Navigate complex documents
- Understand cross-references
- Citation lookup
- Training and onboarding

**Example Usage**:
```
Resolve reference to label "sec:5.6" in document "mk_manual" with context.
```

**What Claude Will Do**:
- Search document for specified label
- Retrieve block content
- Include parent/child context if requested
- Explain what the section covers

---

## ISO 17025 Calibration Prompts (5)

### 11. calculate-measurement-uncertainty

**Purpose**: Calculate measurement uncertainty for calibration data.

**Arguments**:
- `measurement_data` (required): Measurement data array (JSON)
- `instrument_specs` (required): Instrument specifications (JSON)
- `environmental_data` (optional): Environmental conditions (temperature, humidity, pressure)

**Use Cases**:
- Calibration certificate generation
- Uncertainty budget creation
- Method validation
- Compliance with ISO/IEC 17025:2017 clause 7.6

**Example Usage**:
```
Calculate measurement uncertainty for thermometer calibration:
- measurement_data: [20.1, 20.0, 19.9, 20.1, 20.0] °C
- instrument_specs: resolution 0.1°C, accuracy ±0.2°C
- environmental_data: 23°C, 50%RH
```

**What Claude Will Do**:
- Identify uncertainty sources (Type A and Type B)
- Calculate standard uncertainty for each source
- Combine uncertainties (RSS method)
- Calculate expanded uncertainty (k=2)
- Provide detailed uncertainty budget

---

### 12. generate-calibration-hierarchy

**Purpose**: Generate traceability hierarchy for calibration.

**Arguments**:
- `instrument_type` (required): Type of instrument to calibrate
- `measurement_range` (required): Measurement range and units
- `criticality` (optional): Criticality level
  - `high` - Critical measurements (product release)
  - `medium` - Important measurements (process control)
  - `low` - Non-critical measurements (indicative)

**Use Cases**:
- Establish calibration traceability
- Plan calibration chain
- Document metrology infrastructure
- Accreditation scope definition

**Example Usage**:
```
Generate calibration hierarchy for "digital thermometer" with measurement range "-20 to 150°C" at "high" criticality.
```

**What Claude Will Do**:
- Identify required reference standards
- Establish traceability chain to SI units
- Suggest appropriate calibration methods
- Define uncertainty ratios (TUR ≥ 4:1 for high criticality)
- Document calibration intervals

---

### 13. determine-calibration-interval

**Purpose**: Determine optimal calibration interval.

**Arguments**:
- `instrument_data` (required): Instrument usage and history data (JSON)
  - Past calibration results
  - Usage frequency
  - Environmental conditions
  - Failure/drift history
- `risk_tolerance` (optional): Acceptable risk level (default: "medium")

**Use Cases**:
- Optimize calibration schedules
- Balance cost vs. risk
- Comply with ISO 17025:2017 clause 6.4.13
- Resource planning

**Example Usage**:
```
Determine calibration interval for pipette based on:
- 200 uses/month
- Last 3 calibrations: all passed
- No drift detected
- Risk tolerance: low
```

**What Claude Will Do**:
- Analyze historical data
- Calculate drift rate
- Assess usage impact
- Apply risk-based approach
- Recommend interval (e.g., 6, 12, 24 months)
- Justify recommendation

---

### 14. create-calibration-certificate

**Purpose**: Generate calibration certificate from data.

**Arguments**:
- `instrument_data` (required): Instrument information (ID, type, manufacturer, serial number)
- `calibration_results` (required): Calibration results table
- `uncertainty_data` (required): Uncertainty budget data
- `reference_standard` (required): Reference standard details (ID, traceability, cal date)
- `environmental_conditions` (optional): Temp, humidity, pressure during calibration

**Use Cases**:
- Issue calibration certificates
- Document calibration activities
- Meet ISO 17025:2017 clause 7.8 requirements
- Customer deliverables

**Example Usage**:
```
Create calibration certificate for digital thermometer (SN: TH-12345) calibrated on 2024-11-25 at 5 points (-10, 0, 25, 50, 100°C) using reference NIST-traceable PRT (cert #NIST-2024-001).
```

**What Claude Will Do**:
- Generate formatted certificate
- Include all required information per ISO 17025
- Add calibration results table
- State measurement uncertainty
- Reference traceability chain
- Include environmental conditions
- Return certificate in structured format

---

### 15. generate-uncertainty-budget

**Purpose**: Create detailed measurement uncertainty budget.

**Arguments**:
- `measurement_process` (required): Description of measurement process
- `uncertainty_components` (required): List of uncertainty sources (JSON array)
  - Each component: name, type (A/B), value, distribution, sensitivity coefficient

**Use Cases**:
- Method validation
- Uncertainty evaluation per GUM
- ISO 17025:2017 clause 7.6 compliance
- Technical competence demonstration

**Example Usage**:
```
Generate uncertainty budget for temperature measurement process with components:
- Repeatability (Type A): 0.05°C, normal distribution
- Reference thermometer (Type B): 0.10°C, rectangular distribution
- Resolution (Type B): 0.03°C, rectangular distribution
- Drift (Type B): 0.02°C, rectangular distribution
```

**What Claude Will Do**:
- Create detailed uncertainty budget table
- Calculate standard uncertainty for each component
- Apply appropriate divisors (√3 for rectangular, etc.)
- Combine uncertainties (RSS)
- Calculate combined standard uncertainty
- Calculate expanded uncertainty (k=2, ~95% confidence)
- Present in GUM-compliant format

---

## Using Prompts with Claude Desktop

### Basic Usage

1. **Discover Available Prompts**:
   ```
   List all available prompts
   ```

2. **Use a Specific Prompt**:
   ```
   Use the validate-structure prompt on my_document
   ```

3. **Combine Prompts**:
   ```
   First validate-structure on sop_calibration, then suggest-improvements focusing on clarity
   ```

### Advanced Workflows

**Document Quality Review**:
```
1. validate-structure on quality_manual_v3
2. check-consistency on quality_manual_v3
3. validate-compliance (type: quality_manual)
4. suggest-improvements (focus: completeness,compliance)
5. audit-readiness (scope: full)
```

**Calibration Certificate Workflow**:
```
1. calculate-measurement-uncertainty (provide calibration data)
2. generate-uncertainty-budget (from measurement process)
3. create-calibration-certificate (include all results)
```

**Document Development**:
```
1. create-outline (type: sop, topic: "Pipette Calibration")
2. create-section (type: procedure, topic: "Pre-calibration Check")
3. create-section (type: reference, topic: "ISO 8655 Standard")
4. summarize-document (when complete)
```

---

## Best Practices

1. **Always Specify Required Arguments**: Prompts won't work without required arguments

2. **Use Appropriate Document Types**: Match document_type to actual document for best results

3. **Combine Prompts Sequentially**: Build complex workflows by chaining prompts

4. **Verify Results**: Always review Claude's output, especially for compliance-critical work

5. **Provide Context**: Include relevant details in optional arguments for better results

---

## Limitations

- Prompts are templates - actual implementation by Claude may vary
- Compliance checking is advisory, not certification
- Calibration calculations assume valid input data
- Always have human expert review for critical applications

---

## Future Enhancements

- [ ] Custom prompt creation via API
- [ ] Prompt versioning and updates
- [ ] Localization (Hungarian, etc.)
- [ ] Industry-specific prompt packs (Pharma, Aerospace, etc.)
- [ ] Prompt usage analytics

---

## Support

For questions or issues with prompts:
- Review this guide
- Check MCP_COMPLETE_IMPLEMENTATION.md
- Test with example documents first
- Report issues via GitHub

