# TODO: MCP Balanced MVP + Calibration Implementation

**Döntés**: Extended MVP (15 prompt) + 4 dokumentum típus támogatás
- 10 Balanced prompts (általános + ISO 17025 alap)
- 5 Calibration-specific prompts (kalibrálás, mérési bizonytalanság)

**Dokumentum Típusok**:
1. Minőségirányítási kézikönyv (Quality Manual)
2. Eljárások / SOP (Standard Operating Procedures)
3. Munkautasítások (Work Instructions)
4. Audit jelentések (Audit Reports)

---

## Fázis 1: Rust Szerver MCP Metódusok

### 1.1 `initialize` Handler ⏱️ ~30 perc
```rust
// InitializeParams, InitializeResult structs
// Capabilities: tools, resources, prompts
// ServerInfo: "docjl-editor" v0.1.0
// protocolVersion: "2025-06-18"
```

**Taskok**:
- [ ] `InitializeParams` struct
- [ ] `InitializeResult` struct
- [ ] Capabilities definition
- [ ] Handler implementáció main.rs-ben

---

### 1.2 `tools/list` Handler ⏱️ ~45 perc
```rust
// 9 tool definíció áthelyezése Python-ból
// InputSchema minden tool-hoz
```

**Tool Lista** (Python mcp_bridge.py-ból áthozva):
- [ ] mcp_docjl_create_document
- [ ] mcp_docjl_list_documents
- [ ] mcp_docjl_get_document
- [ ] mcp_docjl_list_headings
- [ ] mcp_docjl_search_blocks
- [ ] mcp_docjl_search_content
- [ ] mcp_docjl_insert_block
- [ ] mcp_docjl_update_block
- [ ] mcp_docjl_delete_block

---

### 1.3 `tools/call` Handler ⏱️ ~0 perc
- [x] ✅ Már implementálva
- [x] ✅ Wrapper unwrapping működik
- [x] ✅ Content wrapping response-ban

---

### 1.4 `resources/list` Handler ⏱️ ~30 perc
```rust
// Dokumentumok mint resources
// URI: docjl://document/{doc_id}
```

**Implementáció**:
- [ ] Backend hívás: list_documents
- [ ] Resource array generálás:
  ```json
  {
    "uri": "docjl://document/{id}",
    "name": "{metadata.title}",
    "description": "Auto-generated from metadata",
    "mimeType": "application/json"
  }
  ```
- [ ] Error handling (üres lista)

---

### 1.5 `resources/read` Handler ⏱️ ~30 perc
```rust
// URI parsing: docjl://document/{doc_id}
// Backend: get_document
```

**Implementáció**:
- [ ] URI parsing
- [ ] Backend hívás: get_document
- [ ] Resource response:
  ```json
  {
    "uri": "{original_uri}",
    "mimeType": "application/json",
    "text": "{JSON.stringify(document)}"
  }
  ```
- [ ] 404 handling

---

### 1.6 `prompts/list` Handler ⏱️ ~1.5 óra

#### **Balanced MVP: 10 Prompt**

##### **Általános Prompts (6 db)**

**1. validate-structure** ⏱️ ~10 perc
```json
{
  "name": "validate-structure",
  "description": "Validate DOCJL document structure and label hierarchy",
  "arguments": [
    {
      "name": "document_id",
      "description": "Document ID to validate",
      "required": true
    }
  ]
}
```
**Prompt szöveg**:
```
Analyze the DOCJL document structure for:
1. Label uniqueness (no duplicate labels)
2. Hierarchical correctness (sec:1 → sec:1.1 → sec:1.1.1)
3. Broken references (labels referenced but don't exist)
4. Missing required sections

Return: List of validation errors with severity (error/warning)
```

---

**2. validate-compliance** ⏱️ ~15 perc
```json
{
  "name": "validate-compliance",
  "description": "Check ISO 17025 compliance requirements",
  "arguments": [
    {
      "name": "document_id",
      "description": "Document ID (Quality Manual, SOP, etc.)",
      "required": true
    },
    {
      "name": "document_type",
      "description": "Type: quality_manual | sop | work_instruction | audit_report",
      "required": true
    }
  ]
}
```
**Prompt szöveg**:
```
Check ISO 17025:2017 compliance for {document_type}:

For quality_manual:
- Scope and applicability (4.1)
- Management system (4.2)
- Document control (8.3)
- Control of records (8.4)
- Management review (8.9)

For sop:
- Purpose and scope
- Responsibilities
- Procedure steps
- References to standards
- Version control

For work_instruction:
- Clear step-by-step instructions
- Equipment/materials list
- Safety warnings
- Quality checkpoints

For audit_report:
- Audit scope and criteria
- Audit findings
- Non-conformities
- Corrective actions
- Follow-up plan

Return: Missing sections, non-compliant areas, recommendations
```

---

**3. create-section** ⏱️ ~15 perc
```json
{
  "name": "create-section",
  "description": "Generate new document section from template",
  "arguments": [
    {
      "name": "section_type",
      "description": "Type: quality_manual_section | sop | work_instruction | audit_finding",
      "required": true
    },
    {
      "name": "topic",
      "description": "Section topic/title",
      "required": true
    },
    {
      "name": "context",
      "description": "Additional context or requirements",
      "required": false
    }
  ]
}
```
**Prompt szöveg**:
```
Generate a new {section_type} section about "{topic}".

Use DOCJL format with proper structure:
- heading blocks with labels (sec:X)
- paragraph blocks (para:X)
- proper nesting (children array)

Include based on type:
- quality_manual_section: policy, scope, responsibilities, references
- sop: purpose, scope, procedure steps, forms/records
- work_instruction: materials, step-by-step, checkpoints, troubleshooting
- audit_finding: observation, evidence, requirement reference, severity, recommendation

Return: Complete DOCJL block structure ready for insertion
```

---

**4. summarize-document** ⏱️ ~10 perc
```json
{
  "name": "summarize-document",
  "description": "Create executive summary of document",
  "arguments": [
    {
      "name": "document_id",
      "description": "Document ID to summarize",
      "required": true
    },
    {
      "name": "max_length",
      "description": "Maximum summary length (words)",
      "required": false
    }
  ]
}
```
**Prompt szöveg**:
```
Create an executive summary of the document:
1. Main purpose and scope
2. Key sections and their purpose
3. Critical requirements/procedures
4. Action items or compliance points

Keep summary concise ({max_length} words max if specified).
Focus on actionable information.
```

---

**5. suggest-improvements** ⏱️ ~10 perc
```json
{
  "name": "suggest-improvements",
  "description": "Suggest document quality improvements",
  "arguments": [
    {
      "name": "document_id",
      "description": "Document ID",
      "required": true
    },
    {
      "name": "focus_areas",
      "description": "Comma-separated: clarity, compliance, completeness, consistency",
      "required": false
    }
  ]
}
```
**Prompt szöveg**:
```
Analyze document for improvements in: {focus_areas or "all areas"}

Check for:
- Clarity: Ambiguous language, unclear procedures
- Compliance: ISO 17025 gaps, missing requirements
- Completeness: Missing sections, incomplete procedures
- Consistency: Terminology, formatting, numbering

Return: Prioritized list with specific actionable recommendations
```

---

**6. audit-readiness** ⏱️ ~15 perc
```json
{
  "name": "audit-readiness",
  "description": "Generate ISO 17025 audit readiness checklist",
  "arguments": [
    {
      "name": "document_id",
      "description": "Document ID (typically Quality Manual)",
      "required": true
    },
    {
      "name": "audit_scope",
      "description": "Audit scope sections (e.g., '4,5,6,7,8')",
      "required": false
    }
  ]
}
```
**Prompt szöveg**:
```
Generate ISO 17025:2017 audit readiness checklist for scope: {audit_scope or "all clauses"}

For each requirement:
1. Clause reference (e.g., 4.1.1)
2. Requirement description
3. Evidence location in document (section labels)
4. Status: ✓ Compliant | ⚠ Partial | ✗ Missing
5. Gap description (if not compliant)
6. Recommended action

Prioritize by audit risk (critical/high/medium/low).
Include objective evidence requirements.
```

---

##### **Additional Balanced Prompts (4 db)**

**7. create-outline** ⏱️ ~10 perc
```json
{
  "name": "create-outline",
  "description": "Generate document outline from topic",
  "arguments": [
    {
      "name": "document_type",
      "description": "Type: quality_manual | sop | work_instruction | audit_report",
      "required": true
    },
    {
      "name": "topic",
      "description": "Document topic/title",
      "required": true
    }
  ]
}
```

---

**8. analyze-changes** ⏱️ ~10 perc
```json
{
  "name": "analyze-changes",
  "description": "Compare document versions and highlight changes",
  "arguments": [
    {
      "name": "document_id_old",
      "description": "Old version document ID",
      "required": true
    },
    {
      "name": "document_id_new",
      "description": "New version document ID",
      "required": true
    }
  ]
}
```

---

**9. check-consistency** ⏱️ ~10 perc
```json
{
  "name": "check-consistency",
  "description": "Check terminology, formatting, and numbering consistency",
  "arguments": [
    {
      "name": "document_id",
      "description": "Document ID",
      "required": true
    }
  ]
}
```

---

**10. resolve-reference** ⏱️ ~10 perc
```json
{
  "name": "resolve-reference",
  "description": "Resolve label reference to full block content",
  "arguments": [
    {
      "name": "document_id",
      "description": "Document ID",
      "required": true
    },
    {
      "name": "label",
      "description": "Block label to resolve (e.g., 'sec:4.2.1')",
      "required": true
    },
    {
      "name": "include_context",
      "description": "Include parent/child blocks",
      "required": false
    }
  ]
}
```

---

##### **ISO 17025 Calibration-Specific Prompts (5 db)**

**11. calculate-measurement-uncertainty** ⏱️ ~20 perc
```json
{
  "name": "calculate-measurement-uncertainty",
  "description": "Calculate measurement uncertainty using GUM method",
  "arguments": [
    {
      "name": "measurement_data",
      "description": "Measurement results and conditions",
      "required": true
    },
    {
      "name": "instrument_specs",
      "description": "Instrument specifications (resolution, accuracy)",
      "required": true
    },
    {
      "name": "environmental_data",
      "description": "Environmental conditions (temp, humidity, pressure)",
      "required": false
    }
  ]
}
```
**Prompt szöveg**:
```
Calculate measurement uncertainty according to GUM (Guide to the Expression of Uncertainty in Measurement):

INPUT DATA:
- Measurement data: {measurement_data}
- Instrument specs: {instrument_specs}
- Environmental: {environmental_data}

CALCULATE:
1. Type A uncertainties (statistical - standard deviation of repeated measurements)
2. Type B uncertainties (systematic):
   - Instrument resolution: u(res) = resolution / (2√3)
   - Instrument accuracy: u(acc) = accuracy / √3
   - Temperature effect: u(temp) = coefficient × ΔT / √3
   - Other systematic effects

3. Combined standard uncertainty: u_c = √(Σ u_i²)
4. Expanded uncertainty: U = k × u_c (k=2 for 95% confidence)

OUTPUT:
- Uncertainty budget table (component, value, distribution, divisor, uncertainty)
- Combined standard uncertainty (u_c)
- Expanded uncertainty (U) with coverage factor k
- Uncertainty statement for certificate
```

---

**12. generate-calibration-hierarchy** ⏱️ ~15 perc
```json
{
  "name": "generate-calibration-hierarchy",
  "description": "Generate calibration traceability chain to national/international standards",
  "arguments": [
    {
      "name": "instrument_type",
      "description": "Type of instrument (mass, length, temperature, pressure, etc.)",
      "required": true
    },
    {
      "name": "measurement_range",
      "description": "Measurement range and uncertainty required",
      "required": true
    },
    {
      "name": "country",
      "description": "Country (for national metrology institute)",
      "required": false
    }
  ]
}
```
**Prompt szöveg**:
```
Generate calibration traceability hierarchy for {instrument_type}:

TRACEABILITY CHAIN:
Level 1: International standard
  - SI unit definition (BIPM)
  - Primary realization

Level 2: National standard
  - National Metrology Institute: {determine based on country, e.g., NIST, PTB, NPL}
  - Primary/Secondary standard
  - Uncertainty: [typical NMI uncertainty]

Level 3: Reference standard (laboratory)
  - Calibrated by NMI or accredited lab
  - Reference standard type and ID
  - Uncertainty: [NMI uncertainty × ~2-3]
  - Calibration interval: [recommend based on stability]

Level 4: Working standard (this instrument)
  - Instrument: {instrument_type}
  - Range: {measurement_range}
  - Expected uncertainty: [Level 3 × ~2-3]
  - Calibration interval: [recommend based on usage]

OUTPUT:
- Diagram (ASCII art or description)
- Uncertainty ratios (TUR - Test Uncertainty Ratio ≥ 4:1 recommended)
- Calibration lab requirements (accreditation scope)
- Traceability statement for Quality Manual
```

---

**13. determine-calibration-interval** ⏱️ ~15 perc
```json
{
  "name": "determine-calibration-interval",
  "description": "Determine optimal calibration interval using statistical methods",
  "arguments": [
    {
      "name": "instrument_id",
      "description": "Instrument identifier",
      "required": true
    },
    {
      "name": "historical_data",
      "description": "Previous calibration results (drift data)",
      "required": true
    },
    {
      "name": "usage_frequency",
      "description": "Usage intensity (daily, weekly, monthly)",
      "required": true
    },
    {
      "name": "criticality",
      "description": "Process criticality (high, medium, low)",
      "required": false
    }
  ]
}
```
**Prompt szöveg**:
```
Determine calibration interval for instrument {instrument_id}:

ANALYSIS METHOD:
1. Historical drift analysis
   - Plot calibration values over time
   - Calculate drift rate (change per time unit)
   - Identify trend (linear, exponential, random)

2. Failure rate analysis
   - Count out-of-tolerance findings
   - Calculate probability of being in-tolerance at interval

3. Usage-based adjustment
   - Usage frequency: {usage_frequency}
   - Adjust base interval by usage factor
   - High usage → shorter interval
   - Low usage → consider condition-based monitoring

4. Risk assessment
   - Process criticality: {criticality}
   - Consequences of out-of-tolerance condition
   - Critical processes → shorter interval

RECOMMENDED INTERVAL CALCULATION:
- Base interval from manufacturer: [extract if available]
- Drift-based interval: [time to reach 75% of tolerance]
- Risk-adjusted interval: [apply safety factor based on criticality]

OUTPUT:
- Recommended calibration interval (months/years)
- Statistical confidence level
- Justification and supporting data
- Next review date (typically annual)
- ISO 17025 clause 6.4.13 compliance statement
```

---

**14. create-calibration-certificate** ⏱️ ~20 perc
```json
{
  "name": "create-calibration-certificate",
  "description": "Generate ISO 17025 compliant calibration certificate",
  "arguments": [
    {
      "name": "instrument_data",
      "description": "Instrument details (ID, type, manufacturer, serial)",
      "required": true
    },
    {
      "name": "calibration_results",
      "description": "Measurement results table",
      "required": true
    },
    {
      "name": "uncertainty_data",
      "description": "Uncertainty calculation results",
      "required": true
    },
    {
      "name": "reference_standard",
      "description": "Reference standard used (ID, cert number, due date)",
      "required": true
    },
    {
      "name": "environmental_conditions",
      "description": "Temperature, humidity during calibration",
      "required": true
    }
  ]
}
```
**Prompt szöveg**:
```
Generate calibration certificate per ISO 17025:2017 clause 7.8.2:

CERTIFICATE STRUCTURE (DOCJL format):

sec:1 - Certificate Header
  - Laboratory name and accreditation details
  - Certificate number (unique)
  - Page X of Y
  - Accreditation logo (reference)

sec:2 - Customer Information
  - Customer name and address
  - Contact person

sec:3 - Instrument Under Calibration
  - Description: {instrument_data.type}
  - Manufacturer: {instrument_data.manufacturer}
  - Model: {instrument_data.model}
  - Serial number: {instrument_data.serial}
  - ID number: {instrument_data.id}
  - Location: {instrument_data.location}

sec:4 - Calibration Details
  - Date of calibration
  - Calibration method/procedure reference
  - Environmental conditions: {environmental_conditions}
  - Reference standard: {reference_standard}
    - ID, certificate number, calibration due date

sec:5 - Calibration Results
  - Results table: {calibration_results}
    - Nominal value | Actual reading | Deviation | Tolerance | Pass/Fail
  - Measurement uncertainty: {uncertainty_data.expanded}
  - Coverage factor: k={uncertainty_data.k}
  - Confidence level: {uncertainty_data.confidence}%

sec:6 - Statements
  - Traceability statement to SI units
  - Uncertainty statement (GUM compliance)
  - "This certificate shall not be reproduced except in full..."
  - Accreditation mark usage restrictions

sec:7 - Signatures
  - Calibration technician
  - Technical manager
  - Date of issue

OUTPUT: Complete DOCJL document structure ready for document creation
```

---

**15. generate-uncertainty-budget** ⏱️ ~15 perc
```json
{
  "name": "generate-uncertainty-budget",
  "description": "Create detailed measurement uncertainty budget table",
  "arguments": [
    {
      "name": "measurement_process",
      "description": "Description of measurement process",
      "required": true
    },
    {
      "name": "uncertainty_components",
      "description": "List of uncertainty sources",
      "required": true
    }
  ]
}
```
**Prompt szöveg**:
```
Generate measurement uncertainty budget for: {measurement_process}

UNCERTAINTY COMPONENTS (from {uncertainty_components}):

For each component:
1. Identify source
2. Estimate value
3. Probability distribution (normal, rectangular, triangular, U-shaped)
4. Divisor (√3 for rectangular, 2 for triangular, etc.)
5. Standard uncertainty u(x_i)
6. Sensitivity coefficient c_i
7. Contribution u_i(y) = c_i × u(x_i)

COMMON COMPONENTS:
- Repeatability (Type A): s/√n
- Resolution: resolution/(2√3)
- Reference standard: U_ref/k
- Temperature effect: temp_coeff × ΔT/√3
- Drift: drift_rate × time/√3
- Operator variability: range/√3

UNCERTAINTY BUDGET TABLE (markdown):
| Component | Value | Distribution | Divisor | u(x_i) | c_i | u_i(y) | % Contribution |
|-----------|-------|--------------|---------|--------|-----|--------|----------------|
| ...       | ...   | ...          | ...     | ...    | ... | ...    | ...            |

COMBINED UNCERTAINTY:
- u_c = √(Σ u_i²(y))
- Degrees of freedom (Welch-Satterthwaite if needed)
- Expanded uncertainty: U = k × u_c
- Coverage factor k = 2 (95% confidence)

OUTPUT:
- Formatted table (DOCJL table block if possible, or formatted text)
- Combined and expanded uncertainty
- Dominant contributors highlighted
```

---

## Fázis 2: Python Bridge Egyszerűsítés ⏱️ ~30 perc

### 2.1 MCP Logika Eltávolítása
- [ ] `handle_mcp_protocol()` törlése
- [ ] `initialize` handler törlése
- [ ] `tools/list` handler törlése
- [ ] `tools/call` special handling törlése

### 2.2 "Dumb Proxy" Implementáció
```python
def process_request(request_line: str) -> Dict[str, Any]:
    # 1. Parse JSON
    request = json.loads(request_line)

    # 2. Forward to backend unchanged
    response = requests.post(MCP_SERVER_URL, json=request)

    # 3. Return response unchanged
    return response.json()
```

---

## Fázis 3: Új Tools (Chunking Support) ⏱️ ~1 óra

### 3.1 `mcp_docjl_get_section`
```rust
// Get specific section with children
{
  "document_id": "iso17025",
  "section_label": "sec:4.2.1",
  "include_subsections": true,
  "max_depth": 2
}
```

### 3.2 `mcp_docjl_estimate_tokens`
```rust
// Estimate token count for planning
{
  "document_id": "iso17025",
  "section_label": "sec:4" // optional
}
```

---

## Fázis 4: Tesztelés ⏱️ ~1.5 óra

### 4.1 Unit Tests
- [ ] initialize test
- [ ] tools/list test
- [ ] resources/list test
- [ ] resources/read test
- [ ] prompts/list test (10 prompt check)

### 4.2 Integration Tests
- [ ] Full MCP flow: init → tools → resources → prompts
- [ ] Python bridge passthrough
- [ ] Chunking with large documents

### 4.3 Manual Tests
- [ ] Claude Desktop STDIO
- [ ] HTTP API direct
- [ ] All 4 document types

---

## Időbecslés

| Fázis | Idő |
|-------|-----|
| **1. Rust MCP Methods** | ~4 óra |
| - initialize | 30 perc |
| - tools/list | 45 perc |
| - resources/list | 30 perc |
| - resources/read | 30 perc |
| - prompts/list (10 prompt) | 1.5 óra |
| **2. Python Simplification** | 30 perc |
| **3. New Tools (optional)** | 1 óra |
| **4. Testing** | 1.5 óra |
| **TOTAL** | **~7 óra** |

---

## Megvalósítási Sorrend

1. ✅ **Előkészítés**: TODO frissítés, commit (KÉSZ)
2. **Initialize + Tools/List**: Alapvető MCP metódusok (1.5 óra)
3. **Resources**: List + Read (1 óra)
4. **Prompts**: 10 prompt implementáció (1.5 óra)
5. **Python**: Bridge egyszerűsítés (30 perc)
6. **Testing**: Comprehensive tests (1.5 óra)
7. **Documentation**: Implementation guide (1 óra)

---

## Következő Lépés

**START**: Fázis 1.1 - `initialize` handler implementáció Rust-ban

```bash
# Development workflow
cd /home/petitan/MongoLite/mcp-server
cargo build --release
./test_mcp_initialize.sh
```
