# TODO: MCP Balanced MVP Implementation

**Döntés**: Balanced MVP (10 prompt) + 4 dokumentum típus támogatás

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
