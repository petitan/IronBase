#!/usr/bin/env python3
"""
Import a simple test document via the API
"""
import sys
sys.path.insert(0, 'examples')
from python_client import MCPDocJLClient
import json

client = MCPDocJLClient()

# Create a simple DOCJL document
test_doc = {
    "id": "mk_manual_v1",
    "metadata": {
        "title": "Test Manual",
        "version": "1.0.0",
        "created_at": "2025-01-01T00:00:00Z",
        "modified_at": "2025-01-01T00:00:00Z"
    },
    "docjll": [
        {
            "type": "heading",
            "level": 1,
            "content": [{"type": "text", "content": "Section 1"}],
            "label": "sec:1"
        },
        {
            "type": "paragraph",
            "content": [{"type": "text", "content": "This is paragraph 1"}],
            "label": "para:1"
        },
        {
            "type": "heading",
            "level": 1,
            "content": [{"type": "text", "content": "Section 2"}],
            "label": "sec:2"
        },
        {
            "type": "paragraph",
            "content": [{"type": "text", "content": "This is paragraph 2"}],
            "label": "para:2"
        },
        {
            "type": "heading",
            "level": 1,
            "content": [{"type": "text", "content": "Section 3"}],
            "label": "sec:3"
        },
        {
            "type": "paragraph",
            "content": [{"type": "text", "content": "This is paragraph 3"}],
            "label": "para:3"
        }
    ]
}

# Use raw requests since we don't have an import method in the client
import requests

response = requests.post(
    "http://localhost:8080/import",
    json=test_doc,
    headers={"Content-Type": "application/json"}
)

if response.status_code == 200:
    print("✅ Document imported successfully!")
    print(f"   Response: {response.json()}")
else:
    print(f"❌ Import failed: {response.status_code}")
    print(f"   Response: {response.text}")
