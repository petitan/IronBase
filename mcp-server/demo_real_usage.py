#!/usr/bin/env python3
"""
Real-world usage demo for MCP DOCJL Server
Demonstrates practical document editing scenarios
"""

import requests
import json
from typing import Dict, Any, List

class DOCJLClient:
    """Client for MCP DOCJL Server"""

    def __init__(self, base_url: str = "http://127.0.0.1:8080/mcp"):
        self.base_url = base_url
        self.request_id = 0

    def _call(self, method: str, params: Dict[str, Any]) -> Any:
        """Make JSON-RPC call"""
        self.request_id += 1
        payload = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": self.request_id
        }

        response = requests.post(self.base_url, json=payload)
        result = response.json()

        if "error" in result:
            raise Exception(f"Error: {result['error']}")

        return result.get("result")

    def list_documents(self) -> List[Dict[str, Any]]:
        """List all documents"""
        result = self._call("mcp_docjl_list_documents", {})
        return result.get("documents", [])

    def get_document(self, doc_id: str) -> Dict[str, Any]:
        """Get full document"""
        return self._call("mcp_docjl_get_document", {"document_id": doc_id})

    def get_outline(self, doc_id: str) -> List[Dict[str, Any]]:
        """Get document outline"""
        return self._call("mcp_docjl_list_headings", {"document_id": doc_id})

    def insert_block(self, doc_id: str, block_type: str, content: Any,
                     position: str = "end", label: str = None) -> Dict[str, Any]:
        """Insert new block"""
        params = {
            "document_id": doc_id,
            "block_type": block_type,
            "content": content,
            "position": position
        }
        if label:
            params["label"] = label
        return self._call("mcp_docjl_insert_block", params)

    def search_blocks(self, query: Dict[str, Any]) -> List[Dict[str, Any]]:
        """Search for blocks"""
        return self._call("mcp_docjl_search_blocks", query)

    def update_block(self, doc_id: str, block_label: str,
                     updates: Dict[str, Any]) -> Dict[str, Any]:
        """Update block"""
        return self._call("mcp_docjl_update_block", {
            "document_id": doc_id,
            "block_label": block_label,
            "updates": updates
        })

    def move_block(self, doc_id: str, block_label: str,
                   position: str = "end") -> Dict[str, Any]:
        """Move block"""
        return self._call("mcp_docjl_move_block", {
            "document_id": doc_id,
            "block_label": block_label,
            "position": position
        })

    def delete_block(self, doc_id: str, block_label: str,
                     cascade: bool = False) -> Dict[str, Any]:
        """Delete block"""
        return self._call("mcp_docjl_delete_block", {
            "document_id": doc_id,
            "block_label": block_label,
            "cascade": cascade
        })

    def validate_references(self, doc_id: str) -> Dict[str, Any]:
        """Validate cross-references"""
        return self._call("mcp_docjl_validate_references", {
            "document_id": doc_id
        })

    def get_audit_log(self, limit: int = 10) -> List[Dict[str, Any]]:
        """Get audit log"""
        return self._call("mcp_docjl_get_audit_log", {"limit": limit})


def print_section(title: str):
    """Print section header"""
    print(f"\n{'=' * 60}")
    print(f"  {title}")
    print('=' * 60)


def print_result(label: str, data: Any):
    """Print result with label"""
    print(f"\n{label}:")
    print(json.dumps(data, indent=2))


def demo_scenario_1_browsing(client: DOCJLClient):
    """Scenario 1: Document Browsing & Navigation"""
    print_section("SCENARIO 1: Document Browsing & Navigation")

    # List all documents
    print("\n1. List all available documents...")
    docs = client.list_documents()
    print(f"Found {len(docs)} documents:")
    for doc in docs:
        print(f"  - {doc['id']}: {doc.get('title', 'Untitled')} ({doc.get('blocks_count', 0)} blocks)")

    if not docs:
        print("⚠️  No documents found. Run seed_real_db.py first!")
        return None

    # Get first document
    doc_id = docs[0]['id']
    print(f"\n2. Get full document: {doc_id}...")
    doc = client.get_document(doc_id)
    print(f"Title: {doc.get('title', 'Untitled')}")
    print(f"Version: {doc.get('version', '1.0')}")
    print(f"Blocks: {len(doc.get('docjll', []))}")

    # Get outline
    print(f"\n3. Get document outline...")
    outline = client.get_outline(doc_id)
    print(f"Document structure ({len(outline)} headings):")
    for heading in outline:
        level = heading.get('level', 0)
        indent = "  " * level
        print(f"{indent}- {heading.get('title', 'Untitled')} ({heading.get('label', 'no-label')})")

    return doc_id


def demo_scenario_2_editing(client: DOCJLClient, doc_id: str):
    """Scenario 2: Real Document Editing"""
    print_section("SCENARIO 2: Real Document Editing")

    # Insert new paragraph
    print("\n1. Insert new paragraph...")
    result = client.insert_block(
        doc_id=doc_id,
        block_type="paragraph",
        content=[{"type": "text", "content": "This paragraph was added via MCP API."}],
        position="end"
    )
    print(f"✅ Inserted block: {result.get('affected_labels', [])}")

    # Insert requirement
    print("\n2. Insert new requirement block...")
    result = client.insert_block(
        doc_id=doc_id,
        block_type="requirement",
        content={
            "id": "REQ-001",
            "text": "The system shall respond within 200ms",
            "priority": "HIGH"
        },
        position="end"
    )
    req_label = result.get('affected_labels', [{}])[0].get('new_label', 'unknown')
    print(f"✅ Inserted requirement: {req_label}")

    # Update the requirement
    print("\n3. Update requirement label...")
    result = client.update_block(
        doc_id=doc_id,
        block_label=req_label,
        updates={"label": "req:performance-001"}
    )
    print(f"✅ Updated label: {result.get('affected_labels', [])}")

    return req_label


def demo_scenario_3_search(client: DOCJLClient):
    """Scenario 3: Content Search & Discovery"""
    print_section("SCENARIO 3: Content Search & Discovery")

    # Search by type
    print("\n1. Search for all paragraphs...")
    results = client.search_blocks({"block_type": "paragraph"})
    print(f"Found {len(results)} paragraphs across all documents")

    # Search by content (if implemented)
    print("\n2. Search for blocks mentioning 'requirement'...")
    try:
        results = client.search_blocks({"content_contains": "requirement"})
        print(f"Found {len(results)} blocks mentioning 'requirement'")
    except Exception as e:
        print(f"⚠️  Content search not fully implemented: {e}")


def demo_scenario_4_validation(client: DOCJLClient, doc_id: str):
    """Scenario 4: Validation & Compliance"""
    print_section("SCENARIO 4: Validation & Compliance")

    # Validate references
    print("\n1. Validate cross-references...")
    result = client.validate_references(doc_id)
    if result.get('valid'):
        print(f"✅ All references valid ({len(result.get('references', []))} found)")
    else:
        print(f"❌ Invalid references: {result.get('errors', [])}")

    # Get audit trail
    print("\n2. Get recent audit log...")
    log = client.get_audit_log(limit=5)
    print(f"Last {len(log)} operations:")
    for entry in log:
        print(f"  - {entry.get('timestamp', 'unknown')}: {entry.get('command', 'unknown')}")


def demo_scenario_5_cleanup(client: DOCJLClient, doc_id: str, block_label: str):
    """Scenario 5: Cleanup & Deletion"""
    print_section("SCENARIO 5: Cleanup & Deletion")

    # Delete the test requirement
    print(f"\n1. Delete test block: {block_label}...")
    try:
        result = client.delete_block(doc_id, block_label)
        print(f"✅ Deleted: {result.get('affected_labels', [])}")
    except Exception as e:
        print(f"❌ Delete failed: {e}")


def main():
    """Run all demo scenarios"""
    print("""
╔═══════════════════════════════════════════════════════════════╗
║                                                               ║
║       MCP DOCJL Server - Real-World Usage Demo               ║
║                                                               ║
║  This demo shows practical document editing scenarios        ║
║  using the MCP protocol to interact with DOCJL documents.    ║
║                                                               ║
╚═══════════════════════════════════════════════════════════════╝
    """)

    # Initialize client
    client = DOCJLClient()

    try:
        # Test connection
        print("\n🔗 Connecting to MCP server at http://127.0.0.1:8080/mcp...")
        docs = client.list_documents()
        print("✅ Connected successfully!")

        # Run scenarios
        doc_id = demo_scenario_1_browsing(client)
        if not doc_id:
            return

        req_label = demo_scenario_2_editing(client, doc_id)
        demo_scenario_3_search(client)
        demo_scenario_4_validation(client, doc_id)
        demo_scenario_5_cleanup(client, doc_id, req_label)

        # Final summary
        print_section("DEMO COMPLETE")
        print("""
✅ Successfully demonstrated:
   1. Document browsing and navigation
   2. Block insertion and editing
   3. Content search capabilities
   4. Reference validation
   5. Audit logging
   6. Block deletion

📖 Next steps:
   - Integrate with Claude Desktop (see CLAUDE_DESKTOP_SETUP.md)
   - Try editing real compliance documents
   - Explore advanced features (schema validation, etc.)
        """)

    except requests.exceptions.ConnectionError:
        print("\n❌ Error: Cannot connect to MCP server!")
        print("\nPlease start the server first:")
        print("  cargo run --release --features real-ironbase --bin mcp-docjl-server")
        print("\nOr run in background:")
        print("  cargo run --release --features real-ironbase --bin mcp-docjl-server &")
    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()


if __name__ == "__main__":
    main()
