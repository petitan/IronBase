#!/usr/bin/env python3
"""
MCP DOCJL Python Client Example

This example demonstrates how to interact with the MCP DOCJL server
from Python to perform AI-assisted document editing.
"""

import requests
import json
from typing import Dict, List, Optional, Any


class MCPDocJLClient:
    """Client for MCP DOCJL Server"""

    def __init__(self, base_url: str = "http://localhost:8080", api_key: Optional[str] = None):
        self.base_url = base_url
        self.api_key = api_key
        self.session = requests.Session()

        if api_key:
            self.session.headers.update({"Authorization": f"Bearer {api_key}"})

    def _request(self, method: str, params: Dict[str, Any]) -> Dict[str, Any]:
        """Send MCP JSON-RPC request"""
        response = self.session.post(
            f"{self.base_url}/mcp",
            json={"method": method, "params": params},
            headers={"Content-Type": "application/json"},
        )

        response.raise_for_status()
        data = response.json()

        if "error" in data:
            raise Exception(f"MCP Error: {data['error']}")

        return data.get("result", {})

    def list_documents(self, filter: Optional[Dict] = None) -> List[Dict]:
        """List all DOCJL documents"""
        result = self._request("mcp_docjl_list_documents", {"filter": filter or {}})
        return result.get("documents", [])

    def get_document(
        self, document_id: str, sections: Optional[List[str]] = None, depth: Optional[int] = None
    ) -> Dict:
        """Get a DOCJL document"""
        params = {"document_id": document_id}
        if sections:
            params["sections"] = sections
        if depth is not None:
            params["depth"] = depth

        result = self._request("mcp_docjl_get_document", params)
        return result.get("document", {})

    def insert_block(
        self,
        document_id: str,
        block: Dict,
        parent_label: Optional[str] = None,
        position: str = "end",
        anchor_label: Optional[str] = None,
    ) -> Dict:
        """Insert a new block into a document"""
        params = {
            "document_id": document_id,
            "block": block,
            "position": position,
        }

        if parent_label:
            params["parent_label"] = parent_label
        if anchor_label:
            params["anchor_label"] = anchor_label

        return self._request("mcp_docjl_insert_block", params)

    def update_block(self, document_id: str, block_label: str, updates: Dict) -> Dict:
        """Update an existing block"""
        params = {"document_id": document_id, "block_label": block_label, "updates": updates}

        return self._request("mcp_docjl_update_block", params)

    def move_block(
        self,
        document_id: str,
        block_label: str,
        target_parent: Optional[str] = None,
        position: str = "end",
    ) -> Dict:
        """Move a block to a new location"""
        params = {"document_id": document_id, "block_label": block_label, "position": position}

        if target_parent:
            params["target_parent"] = target_parent

        return self._request("mcp_docjl_move_block", params)

    def delete_block(self, document_id: str, block_label: str, cascade: bool = False, force: bool = False) -> Dict:
        """Delete a block"""
        params = {
            "document_id": document_id,
            "block_label": block_label,
            "cascade": cascade,
            "force": force,
        }

        return self._request("mcp_docjl_delete_block", params)

    def list_headings(self, document_id: str, max_depth: Optional[int] = None) -> List[Dict]:
        """Get document outline (table of contents)"""
        params = {"document_id": document_id}
        if max_depth is not None:
            params["max_depth"] = max_depth

        result = self._request("mcp_docjl_list_headings", params)
        return result.get("outline", [])

    def search_blocks(
        self,
        document_id: str,
        block_type: Optional[str] = None,
        content_contains: Optional[str] = None,
        has_label: Optional[bool] = None,
        has_compliance_note: Optional[bool] = None,
        label_prefix: Optional[str] = None,
    ) -> List[Dict]:
        """Search for blocks in a document"""
        query = {}

        if block_type:
            query["type"] = block_type
        if content_contains:
            query["content_contains"] = content_contains
        if has_label is not None:
            query["has_label"] = has_label
        if has_compliance_note is not None:
            query["has_compliance_note"] = has_compliance_note
        if label_prefix:
            query["label_prefix"] = label_prefix

        params = {"document_id": document_id, "query": query}

        result = self._request("mcp_docjl_search_blocks", params)
        return result.get("results", [])

    def validate_references(self, document_id: str) -> Dict:
        """Validate all cross-references in a document"""
        params = {"document_id": document_id}
        return self._request("mcp_docjl_validate_references", params)

    def validate_schema(self, document_id: str) -> Dict:
        """Validate document against DOCJL schema"""
        params = {"document_id": document_id}
        return self._request("mcp_docjl_validate_schema", params)

    def get_audit_log(
        self,
        document_id: Optional[str] = None,
        block_label: Optional[str] = None,
        limit: Optional[int] = None,
    ) -> List[Dict]:
        """Get audit log entries"""
        params = {}

        if document_id:
            params["document_id"] = document_id
        if block_label:
            params["block_label"] = block_label
        if limit:
            params["limit"] = limit

        result = self._request("mcp_docjl_get_audit_log", params)
        return result.get("entries", [])

    def health_check(self) -> Dict:
        """Check server health"""
        response = self.session.get(f"{self.base_url}/health")
        response.raise_for_status()
        return response.json()


# ============================================================================
# Example Usage
# ============================================================================


def example_basic_operations():
    """Example: Basic DOCJL operations"""
    client = MCPDocJLClient(api_key="test_key_12345")

    print("=== Health Check ===")
    health = client.health_check()
    print(f"Server status: {health['status']}, version: {health['version']}")

    print("\n=== List Documents ===")
    documents = client.list_documents()
    print(f"Found {len(documents)} documents")

    if documents:
        doc_id = documents[0]["id"]
        print(f"\n=== Get Document: {doc_id} ===")
        document = client.get_document(doc_id)
        print(f"Title: {document.get('title')}")
        print(f"Blocks: {document.get('blocks_count', 0)}")


def example_insert_paragraph():
    """Example: Insert a new paragraph"""
    client = MCPDocJLClient(api_key="test_key_12345")

    # Create a paragraph block
    paragraph = {
        "type": "paragraph",
        "content": [
            {"type": "text", "content": "This is a new requirement: "},
            {"type": "bold", "content": "All calibration must be traceable to national standards."},
        ],
        "compliance_note": "ISO 17025:2018 Section 6.5.1",
    }

    result = client.insert_block(
        document_id="doc_123",
        block=paragraph,
        parent_label="sec:6",  # Insert in section 6
        position="end",  # At the end
    )

    print(f"Inserted block: {result['block_label']}")
    print(f"Audit ID: {result['audit_id']}")


def example_insert_table():
    """Example: Insert a table"""
    client = MCPDocJLClient(api_key="test_key_12345")

    table = {
        "type": "table",
        "headers": ["Equipment", "Calibration Interval", "Last Calibration"],
        "rows": [
            ["Digital Multimeter", "12 months", "2024-01-15"],
            ["Oscilloscope", "12 months", "2024-02-01"],
            ["Temperature Probe", "6 months", "2024-06-01"],
        ],
        "caption": "Calibration Schedule",
    }

    result = client.insert_block(document_id="doc_123", block=table, position="end")

    print(f"Inserted table: {result['block_label']}")


def example_update_block():
    """Example: Update an existing block"""
    client = MCPDocJLClient(api_key="test_key_12345")

    # Update paragraph content
    updates = {
        "content": [
            {"type": "text", "content": "Updated text: "},
            {"type": "bold", "content": "Revised calibration procedure."},
        ]
    }

    result = client.update_block(document_id="doc_123", block_label="para:6.2", updates=updates)

    print(f"Updated block: {result['audit_id']}")


def example_move_block():
    """Example: Move a block to a different section"""
    client = MCPDocJLClient(api_key="test_key_12345")

    result = client.move_block(
        document_id="doc_123",
        block_label="para:4.5",
        target_parent="sec:5",  # Move to section 5
        position="end",
    )

    print(f"Moved block. Affected labels: {result['affected_labels']}")


def example_get_outline():
    """Example: Get document outline"""
    client = MCPDocJLClient(api_key="test_key_12345")

    outline = client.list_headings(document_id="doc_123", max_depth=2)

    print("=== Document Outline ===")
    for item in outline:
        indent = "  " * (item["level"] - 1)
        print(f"{indent}{item['level']}. {item['title']} ({item['label']})")


def example_search_blocks():
    """Example: Search for blocks"""
    client = MCPDocJLClient(api_key="test_key_12345")

    # Search for paragraphs containing "calibration"
    results = client.search_blocks(
        document_id="doc_123", block_type="paragraph", content_contains="calibration"
    )

    print(f"=== Search Results ({len(results)} found) ===")
    for result in results[:5]:  # Show first 5
        print(f"Label: {result['label']}")
        print(f"Path: {' > '.join(result['path'])}")
        print(f"Score: {result['score']}")
        print()


def example_validate_document():
    """Example: Validate document"""
    client = MCPDocJLClient(api_key="test_key_12345")

    print("=== Schema Validation ===")
    schema_result = client.validate_schema(document_id="doc_123")
    print(f"Valid: {schema_result['valid']}")
    if schema_result["errors"]:
        print("Errors:")
        for error in schema_result["errors"]:
            print(f"  - {error['message']}")

    print("\n=== Reference Validation ===")
    ref_result = client.validate_references(document_id="doc_123")
    print(f"Valid: {ref_result['valid']}")
    if ref_result["errors"]:
        print("Broken references:")
        for error in ref_result["errors"]:
            print(f"  - {error['message']}")


def example_audit_log():
    """Example: Get audit log"""
    client = MCPDocJLClient(api_key="test_key_12345")

    # Get recent audit entries for a document
    entries = client.get_audit_log(document_id="doc_123", limit=10)

    print(f"=== Recent Changes ({len(entries)} entries) ===")
    for entry in entries:
        print(f"[{entry['timestamp']}] {entry['command']} by {entry['api_key_name']}")
        if entry.get("block_label"):
            print(f"  Block: {entry['block_label']}")
        print(f"  Result: {entry['result']['status']}")
        print()


def example_ai_workflow():
    """Example: Simulated AI workflow"""
    client = MCPDocJLClient(api_key="test_key_12345")

    print("=== AI Document Editing Workflow ===\n")

    # Step 1: Get document outline
    print("1. Getting document outline...")
    outline = client.list_headings("doc_123", max_depth=2)
    print(f"   Document has {len(outline)} top-level sections\n")

    # Step 2: Find section for new content
    print("2. Searching for 'Quality Control' section...")
    results = client.search_blocks(
        document_id="doc_123", block_type="heading", content_contains="Quality Control"
    )

    if results:
        qc_section = results[0]["label"]
        print(f"   Found: {qc_section}\n")

        # Step 3: Insert new procedure
        print("3. Inserting new calibration procedure...")
        paragraph = {
            "type": "paragraph",
            "content": [
                {
                    "type": "text",
                    "content": "All measurement equipment shall be calibrated at regular intervals as defined in ",
                },
                {"type": "ref", "target": "tab:5"},
                {"type": "text", "content": "."},
            ],
        }

        result = client.insert_block(
            document_id="doc_123", block=paragraph, parent_label=qc_section, position="end"
        )
        new_label = result["block_label"]
        print(f"   Inserted: {new_label}\n")

        # Step 4: Validate references
        print("4. Validating cross-references...")
        validation = client.validate_references("doc_123")
        if validation["valid"]:
            print("   ✓ All references valid\n")
        else:
            print("   ✗ Broken references found\n")

        # Step 5: Get audit trail
        print("5. Recording changes in audit log...")
        audit_entries = client.get_audit_log(block_label=new_label, limit=1)
        if audit_entries:
            print(f"   Audit ID: {audit_entries[0]['audit_id']}")

    print("\n=== Workflow Complete ===")


if __name__ == "__main__":
    print("MCP DOCJL Python Client Examples\n")
    print("=" * 70)

    # Run examples (uncomment as needed)
    try:
        example_basic_operations()
        # example_insert_paragraph()
        # example_insert_table()
        # example_update_block()
        # example_move_block()
        # example_get_outline()
        # example_search_blocks()
        # example_validate_document()
        # example_audit_log()
        # example_ai_workflow()

    except Exception as e:
        print(f"\nError: {e}")
        print("\nMake sure the MCP DOCJL server is running:")
        print("  cd mcp-server && cargo run")
