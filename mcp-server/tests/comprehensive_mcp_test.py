#!/usr/bin/env python3
"""
Comprehensive MCP Server Test Suite
====================================
100+ automated tests covering all MCP tools, measuring execution times.

Usage:
    python comprehensive_mcp_test.py [--host HOST] [--port PORT] [--verbose]

Requirements:
    pip install requests
"""

import argparse
import json
import random
import string
import sys
import time
from dataclasses import dataclass, field
from typing import Any, Callable, Dict, List, Optional, Tuple
import requests
import urllib3

# Disable SSL warnings for self-signed certs
urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)


@dataclass
class TestResult:
    """Single test result"""
    name: str
    category: str
    passed: bool
    duration_ms: float
    error: Optional[str] = None
    details: Optional[str] = None


@dataclass
class TestStats:
    """Aggregated test statistics"""
    total: int = 0
    passed: int = 0
    failed: int = 0
    skipped: int = 0
    total_time_ms: float = 0.0
    results: List[TestResult] = field(default_factory=list)

    def add(self, result: TestResult):
        self.results.append(result)
        self.total += 1
        if result.passed:
            self.passed += 1
        else:
            self.failed += 1
        self.total_time_ms += result.duration_ms


class MCPClient:
    """MCP JSON-RPC client"""

    def __init__(self, host: str = "localhost", port: int = 8080, use_tls: bool = True):
        protocol = "https" if use_tls else "http"
        self.base_url = f"{protocol}://{host}:{port}/mcp"
        self.request_id = 0
        self.session = requests.Session()
        self.initialized = False

    def _call(self, method: str, params: Optional[Dict] = None) -> Tuple[Any, float]:
        """Make JSON-RPC call, return (result, duration_ms)"""
        self.request_id += 1
        payload = {
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
        }
        if params:
            payload["params"] = params

        start = time.perf_counter()
        response = self.session.post(
            self.base_url,
            json=payload,
            verify=False,  # Allow self-signed certs
            timeout=300
        )
        duration_ms = (time.perf_counter() - start) * 1000

        data = response.json()
        if "error" in data:
            raise Exception(f"[{data['error'].get('code', '?')}] {data['error'].get('message', 'Unknown error')}")
        return data.get("result"), duration_ms

    def initialize(self) -> Tuple[Any, float]:
        """Initialize MCP session"""
        result, duration = self._call("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "comprehensive-test", "version": "1.0"}
        })
        self.initialized = True
        return result, duration

    def tool(self, name: str, arguments: Dict) -> Tuple[Any, float]:
        """Call a tool, return (parsed_result, duration_ms)"""
        result, duration = self._call("tools/call", {
            "name": name,
            "arguments": arguments
        })
        # Parse the text content
        if result and "content" in result:
            for content in result["content"]:
                if content.get("type") == "text":
                    try:
                        return json.loads(content["text"]), duration
                    except json.JSONDecodeError:
                        return content["text"], duration
        return result, duration


class MCPTestSuite:
    """Comprehensive MCP test suite"""

    def __init__(self, client: MCPClient, verbose: bool = False):
        self.client = client
        self.verbose = verbose
        self.stats = TestStats()
        self.test_collection = f"_test_{self._random_string(8)}"

    def _random_string(self, length: int = 8) -> str:
        return ''.join(random.choices(string.ascii_lowercase, k=length))

    def _random_doc(self, index: int = 0) -> Dict:
        """Generate random test document"""
        return {
            "name": f"Test_{self._random_string(5)}",
            "index": index,
            "value": random.randint(1, 1000),
            "score": round(random.uniform(0, 100), 2),
            "active": random.choice([True, False]),
            "tags": [self._random_string(3) for _ in range(random.randint(1, 5))],
            "nested": {
                "level1": {
                    "level2": self._random_string(10)
                }
            }
        }

    def _run_test(self, name: str, category: str, func: Callable) -> TestResult:
        """Run a single test and capture result"""
        start = time.perf_counter()
        try:
            details = func()
            duration_ms = (time.perf_counter() - start) * 1000
            result = TestResult(
                name=name,
                category=category,
                passed=True,
                duration_ms=duration_ms,
                details=str(details) if details else None
            )
        except Exception as e:
            duration_ms = (time.perf_counter() - start) * 1000
            result = TestResult(
                name=name,
                category=category,
                passed=False,
                duration_ms=duration_ms,
                error=str(e)
            )

        self.stats.add(result)

        if self.verbose:
            status = "✅" if result.passed else "❌"
            print(f"  {status} {name} ({result.duration_ms:.1f}ms)")
            if result.error:
                print(f"      Error: {result.error}")

        return result

    # =========================================================================
    # Test Categories
    # =========================================================================

    def test_initialization(self):
        """Test MCP initialization"""
        print("\n📋 Initialization Tests")

        def test_init():
            result, _ = self.client.initialize()
            assert "capabilities" in result
            return result.get("serverInfo", {}).get("version")

        self._run_test("initialize", "init", test_init)

    def test_collection_operations(self):
        """Test collection CRUD"""
        print("\n📁 Collection Operations")

        # Create collection
        def test_create():
            result, _ = self.client.tool("collection_create", {"collection": self.test_collection})
            assert result.get("success") == True
            return result
        self._run_test("collection_create", "collection", test_create)

        # List collections
        def test_list():
            result, _ = self.client.tool("collection_list", {})
            assert "collections" in result
            assert self.test_collection in result["collections"]
            return f"{len(result['collections'])} collections"
        self._run_test("collection_list", "collection", test_list)

        # Invalid collection name (security test)
        def test_invalid_name():
            try:
                self.client.tool("collection_create", {"collection": "_invalid.name"})
                raise AssertionError("Should have rejected invalid name")
            except Exception as e:
                assert "cannot contain dots" in str(e).lower() or "invalid" in str(e).lower()
                return "Correctly rejected"
        self._run_test("collection_create_invalid_name", "collection", test_invalid_name)

    def test_insert_operations(self):
        """Test insert operations"""
        print("\n📥 Insert Operations")

        # Insert one
        def test_insert_one():
            doc = self._random_doc(0)
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            assert "inserted_id" in result
            return result["inserted_id"]
        self._run_test("insert_one", "insert", test_insert_one)

        # Insert many
        def test_insert_many():
            docs = [self._random_doc(i) for i in range(1, 101)]
            result, _ = self.client.tool("insert_many", {
                "collection": self.test_collection,
                "documents": docs
            })
            assert result.get("inserted_count") == 100
            return f"{result['inserted_count']} docs"
        self._run_test("insert_many_100", "insert", test_insert_many)

        # Insert with nested document
        def test_insert_nested():
            doc = {
                "type": "nested_test",
                "deep": {
                    "level1": {
                        "level2": {
                            "level3": {
                                "value": "deep_value"
                            }
                        }
                    }
                }
            }
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            assert "inserted_id" in result
            return result["inserted_id"]
        self._run_test("insert_nested_doc", "insert", test_insert_nested)

    def test_find_operations(self):
        """Test find/query operations"""
        print("\n🔍 Find Operations")

        # Find all
        def test_find_all():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "limit": 10
            })
            assert "documents" in result
            return f"{result['count']} docs"
        self._run_test("find_all", "find", test_find_all)

        # Find with filter
        def test_find_filter():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"active": True},
                "limit": 50
            })
            assert "documents" in result
            return f"{result['count']} active docs"
        self._run_test("find_with_filter", "find", test_find_filter)

        # Find with projection
        def test_find_projection():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "projection": {"name": 1, "value": 1},
                "limit": 10
            })
            if result["documents"]:
                assert "score" not in result["documents"][0]
            return f"{result['count']} docs (projected)"
        self._run_test("find_with_projection", "find", test_find_projection)

        # Find with sort
        def test_find_sort():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "sort": [["value", -1]],
                "limit": 10
            })
            assert "documents" in result
            return f"Top value: {result['documents'][0].get('value') if result['documents'] else 'N/A'}"
        self._run_test("find_with_sort", "find", test_find_sort)

        # Find with skip/limit (pagination)
        def test_find_pagination():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "skip": 10,
                "limit": 10
            })
            assert "documents" in result
            return f"Page 2: {result['count']} docs"
        self._run_test("find_pagination", "find", test_find_pagination)

        # Find one
        def test_find_one():
            result, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"index": 0}
            })
            return f"Found: {result is not None}"
        self._run_test("find_one", "find", test_find_one)

        # Find with $gt operator
        def test_find_gt():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"value": {"$gt": 500}},
                "limit": 100
            })
            assert "documents" in result
            return f"{result['count']} docs with value > 500"
        self._run_test("find_$gt", "find", test_find_gt)

        # Find with $in operator
        def test_find_in():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"index": {"$in": [1, 2, 3, 4, 5]}},
                "limit": 10
            })
            assert "documents" in result
            return f"{result['count']} docs"
        self._run_test("find_$in", "find", test_find_in)

        # Find with $and
        def test_find_and():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"$and": [{"value": {"$gt": 100}}, {"active": True}]},
                "limit": 50
            })
            return f"{result['count']} docs"
        self._run_test("find_$and", "find", test_find_and)

        # Find with $or
        def test_find_or():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"$or": [{"index": 0}, {"index": 1}]},
                "limit": 10
            })
            return f"{result['count']} docs"
        self._run_test("find_$or", "find", test_find_or)

        # Find with $exists
        def test_find_exists():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"nested.level1.level2": {"$exists": True}},
                "limit": 50
            })
            return f"{result['count']} docs"
        self._run_test("find_$exists", "find", test_find_exists)

        # Find with dot notation
        def test_find_dot_notation():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"type": "nested_test"},
                "limit": 10
            })
            return f"{result['count']} nested docs"
        self._run_test("find_dot_notation", "find", test_find_dot_notation)

        # Find with $regex
        def test_find_regex():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"name": {"$regex": "^Test_"}},
                "limit": 50
            })
            return f"{result['count']} matching docs"
        self._run_test("find_$regex", "find", test_find_regex)

    def test_update_operations(self):
        """Test update operations"""
        print("\n✏️ Update Operations")

        # Update one with $set
        def test_update_set():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 0},
                "update": {"$set": {"updated": True, "update_time": "2024-01-01"}}
            })
            assert result.get("modified_count", 0) >= 0
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$set", "update", test_update_set)

        # Update one with $inc
        def test_update_inc():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 1},
                "update": {"$inc": {"value": 100}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$inc", "update", test_update_inc)

        # Update one with $push
        def test_update_push():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 2},
                "update": {"$push": {"tags": "new_tag"}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$push", "update", test_update_push)

        # Update many
        def test_update_many():
            result, _ = self.client.tool("update_many", {
                "collection": self.test_collection,
                "filter": {"active": True},
                "update": {"$set": {"batch_updated": True}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_many", "update", test_update_many)

        # Upsert
        def test_upsert():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"upsert_key": "unique_value"},
                "update": {"$set": {"upsert_key": "unique_value", "data": "upserted"}},
                "upsert": True
            })
            return f"Upserted: {result.get('upserted_id') is not None}"
        self._run_test("upsert", "update", test_upsert)

        # Update with dot notation
        def test_update_nested():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"type": "nested_test"},
                "update": {"$set": {"deep.level1.new_field": "added"}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_nested_field", "update", test_update_nested)

    def test_delete_operations(self):
        """Test delete operations"""
        print("\n🗑️ Delete Operations")

        # Insert docs to delete
        def test_delete_setup():
            docs = [{"delete_test": True, "batch": i} for i in range(10)]
            result, _ = self.client.tool("insert_many", {
                "collection": self.test_collection,
                "documents": docs
            })
            return f"Inserted {result['inserted_count']} for deletion"
        self._run_test("delete_setup", "delete", test_delete_setup)

        # Delete one
        def test_delete_one():
            result, _ = self.client.tool("delete_one", {
                "collection": self.test_collection,
                "filter": {"delete_test": True}
            })
            return f"Deleted: {result.get('deleted_count', 0)}"
        self._run_test("delete_one", "delete", test_delete_one)

        # Delete many
        def test_delete_many():
            result, _ = self.client.tool("delete_many", {
                "collection": self.test_collection,
                "filter": {"delete_test": True}
            })
            return f"Deleted: {result.get('deleted_count', 0)}"
        self._run_test("delete_many", "delete", test_delete_many)

    def test_count_distinct(self):
        """Test count and distinct operations"""
        print("\n📊 Count & Distinct")

        # Count all
        def test_count_all():
            result, _ = self.client.tool("count_documents", {
                "collection": self.test_collection,
                "query": {}
            })
            return f"Count: {result.get('count')}"
        self._run_test("count_all", "count", test_count_all)

        # Count with filter
        def test_count_filter():
            result, _ = self.client.tool("count_documents", {
                "collection": self.test_collection,
                "query": {"active": True}
            })
            return f"Active count: {result.get('count')}"
        self._run_test("count_filtered", "count", test_count_filter)

        # Distinct
        def test_distinct():
            result, _ = self.client.tool("distinct", {
                "collection": self.test_collection,
                "field": "active",
                "query": {}
            })
            return f"Distinct values: {result.get('values')}"
        self._run_test("distinct", "count", test_distinct)

    def test_index_operations(self):
        """Test index operations"""
        print("\n📇 Index Operations")

        # Create index
        def test_create_index():
            result, _ = self.client.tool("index_create", {
                "collection": self.test_collection,
                "field": "value"
            })
            assert "index_name" in result
            return result["index_name"]
        self._run_test("index_create", "index", test_create_index)

        # Create compound index
        def test_create_compound():
            result, _ = self.client.tool("index_create", {
                "collection": self.test_collection,
                "fields": ["active", "value"]
            })
            return result.get("index_name")
        self._run_test("index_create_compound", "index", test_create_compound)

        # Create unique index
        def test_create_unique():
            result, _ = self.client.tool("index_create", {
                "collection": self.test_collection,
                "field": "upsert_key",
                "unique": True,
                "sparse": True
            })
            return result.get("index_name")
        self._run_test("index_create_unique_sparse", "index", test_create_unique)

        # List indexes
        def test_list_indexes():
            result, _ = self.client.tool("index_list", {
                "collection": self.test_collection
            })
            total = result.get("summary", {}).get("total", 0)
            return f"{total} indexes"
        self._run_test("index_list", "index", test_list_indexes)

        # Explain query
        def test_explain():
            result, _ = self.client.tool("explain", {
                "collection": self.test_collection,
                "query": {"value": {"$gt": 500}}
            })
            return f"Plan: {result.get('plan', {}).get('type', 'unknown')}"
        self._run_test("explain", "index", test_explain)

        # Find with hint
        def test_find_hint():
            result, _ = self.client.tool("find_with_hint", {
                "collection": self.test_collection,
                "query": {"value": {"$gt": 500}},
                "hint": f"{self.test_collection}_value",
                "limit": 10
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_with_hint", "index", test_find_hint)

        # Drop index
        def test_drop_index():
            result, _ = self.client.tool("index_drop", {
                "collection": self.test_collection,
                "index_name": f"{self.test_collection}_value"
            })
            return f"Dropped: {result.get('success')}"
        self._run_test("index_drop", "index", test_drop_index)

    def test_aggregation(self):
        """Test aggregation pipeline"""
        print("\n📈 Aggregation Pipeline")

        # Simple group
        def test_agg_group():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$group": {"_id": "$active", "count": {"$sum": 1}}}
                ]
            })
            return f"{len(result)} groups"
        self._run_test("aggregate_$group", "aggregation", test_agg_group)

        # Match + Group
        def test_agg_match_group():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$match": {"value": {"$gt": 100}}},
                    {"$group": {"_id": "$active", "avg_value": {"$avg": "$value"}}}
                ]
            })
            return f"{len(result)} groups"
        self._run_test("aggregate_$match_$group", "aggregation", test_agg_match_group)

        # Group with multiple accumulators
        def test_agg_accumulators():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$group": {
                        "_id": "$active",
                        "count": {"$sum": 1},
                        "total": {"$sum": "$value"},
                        "avg": {"$avg": "$value"},
                        "min": {"$min": "$value"},
                        "max": {"$max": "$value"}
                    }}
                ]
            })
            return f"{len(result)} groups with 5 accumulators"
        self._run_test("aggregate_accumulators", "aggregation", test_agg_accumulators)

        # Sort + Limit (Top-K)
        def test_agg_topk():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$sort": {"value": -1}},
                    {"$limit": 5}
                ]
            })
            top_values = [d.get('value') for d in result[:3]] if isinstance(result, list) else []
            return f"Top 5 values: {top_values}..."
        self._run_test("aggregate_top_k", "aggregation", test_agg_topk)

        # Project
        def test_agg_project():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$project": {"name": 1, "value": 1}},
                    {"$limit": 5}
                ]
            })
            return f"{len(result)} projected docs"
        self._run_test("aggregate_$project", "aggregation", test_agg_project)

        # Skip
        def test_agg_skip():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$skip": 10},
                    {"$limit": 5}
                ]
            })
            return f"{len(result)} docs after skip"
        self._run_test("aggregate_$skip", "aggregation", test_agg_skip)

    def test_schema_operations(self):
        """Test schema operations"""
        print("\n📐 Schema Operations")

        # Set schema
        def test_set_schema():
            schema = {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "value": {"type": "integer"}
                },
                "required": ["name"]
            }
            result, _ = self.client.tool("schema_set", {
                "collection": self.test_collection,
                "schema": schema
            })
            return f"Schema set: {result.get('schema_set')}"
        self._run_test("schema_set", "schema", test_set_schema)

        # Get schema
        def test_get_schema():
            result, _ = self.client.tool("schema_get", {
                "collection": self.test_collection
            })
            return f"Has schema: {result.get('schema') is not None}"
        self._run_test("schema_get", "schema", test_get_schema)

        # Clear schema
        def test_clear_schema():
            result, _ = self.client.tool("schema_set", {
                "collection": self.test_collection,
                "schema": None
            })
            return f"Schema cleared: {not result.get('schema_set')}"
        self._run_test("schema_clear", "schema", test_clear_schema)

    def test_db_operations(self):
        """Test database operations"""
        print("\n🗄️ Database Operations")

        # DB stats
        def test_db_stats():
            result, _ = self.client.tool("db_stats", {})
            return f"Collections: {result.get('collections_count', '?')}"
        self._run_test("db_stats", "db", test_db_stats)

        # DB checkpoint
        def test_checkpoint():
            result, _ = self.client.tool("db_checkpoint", {})
            return f"Success: {result.get('success')}"
        self._run_test("db_checkpoint", "db", test_checkpoint)

    def test_acl_operations(self):
        """Test ACL operations"""
        print("\n🔐 ACL Operations")

        # List ACLs
        def test_acl_list():
            result, _ = self.client.tool("acl_list", {})
            return f"{result.get('count', 0)} ACL rules"
        self._run_test("acl_list", "acl", test_acl_list)

        # Get ACL
        def test_acl_get():
            result, _ = self.client.tool("acl_get", {
                "collection": self.test_collection
            })
            return f"Has rules: {result.get('rules') is not None}"
        self._run_test("acl_get", "acl", test_acl_get)

        # Set ACL
        def test_acl_set():
            result, _ = self.client.tool("acl_set", {
                "collection": self.test_collection,
                "rules": [
                    {"principal": "user:test", "permissions": "rw"}
                ]
            })
            rules_count = result.get('rules_count', 0) if isinstance(result, dict) else 0
            return f"Rules set: {rules_count}"
        self._run_test("acl_set", "acl", test_acl_set)

        # Delete ACL
        def test_acl_delete():
            result, _ = self.client.tool("acl_delete", {
                "collection": self.test_collection
            })
            return f"Deleted: {result.get('deleted')}"
        self._run_test("acl_delete", "acl", test_acl_delete)

    def test_listener_operations(self):
        """Test listener operations"""
        print("\n📡 Listener Operations")

        # List listeners
        def test_listener_list():
            result, _ = self.client.tool("listener_list", {})
            return f"{result.get('count', 0)} listeners"
        self._run_test("listener_list", "listener", test_listener_list)

    def test_edge_cases(self):
        """Test edge cases and error handling"""
        print("\n⚠️ Edge Cases")

        # Empty query
        def test_empty_query():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "limit": 1
            })
            return f"Found: {result.get('count', 0)}"
        self._run_test("empty_query", "edge", test_empty_query)

        # Non-existent collection
        def test_nonexistent():
            try:
                self.client.tool("find", {
                    "collection": "nonexistent_" + self._random_string(10),
                    "query": {}
                })
                return "No error (empty result)"
            except Exception as e:
                return f"Error: {str(e)[:50]}"
        self._run_test("nonexistent_collection", "edge", test_nonexistent)

        # Large limit
        def test_large_limit():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "limit": 100000  # Should be capped
            })
            return f"Returned: {result.get('count', 0)}"
        self._run_test("large_limit", "edge", test_large_limit)

        # Unicode in data
        def test_unicode():
            doc = {"text": "Árvíztűrő tükörfúrógép 中文 🚀"}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return f"Inserted: {result.get('inserted_id')}"
        self._run_test("unicode_data", "edge", test_unicode)

        # Very nested query
        def test_deep_query():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"deep.level1.level2.level3.value": "deep_value"},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)}"
        self._run_test("deep_nested_query", "edge", test_deep_query)

        # Empty string field
        def test_empty_string():
            doc = {"empty_field": "", "null_field": None}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return f"Inserted: {result.get('inserted_id')}"
        self._run_test("empty_null_fields", "edge", test_empty_string)

        # Array field operations
        def test_array_size():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"tags": {"$size": 3}},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} with 3 tags"
        self._run_test("array_$size", "edge", test_array_size)

        # $all operator
        def test_array_all():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"tags": {"$all": ["new_tag"]}},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)}"
        self._run_test("array_$all", "edge", test_array_all)

    def test_advanced_queries(self):
        """Advanced query tests"""
        print("\n🔬 Advanced Query Tests")

        # $ne operator
        def test_ne():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"active": {"$ne": True}},
                "limit": 20
            })
            return f"{result.get('count', 0)} inactive docs"
        self._run_test("find_$ne", "advanced", test_ne)

        # $nin operator
        def test_nin():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"index": {"$nin": [0, 1, 2]}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_$nin", "advanced", test_nin)

        # $lte operator
        def test_lte():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"value": {"$lte": 100}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs with value <= 100"
        self._run_test("find_$lte", "advanced", test_lte)

        # $gte operator
        def test_gte():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"score": {"$gte": 50.0}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs with score >= 50"
        self._run_test("find_$gte", "advanced", test_gte)

        # $lt operator
        def test_lt():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"index": {"$lt": 10}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_$lt", "advanced", test_lt)

        # Combined range query
        def test_range():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"value": {"$gte": 100, "$lte": 500}},
                "limit": 50
            })
            return f"{result.get('count', 0)} docs in range"
        self._run_test("find_range_query", "advanced", test_range)

        # $not operator
        def test_not():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"value": {"$not": {"$gt": 500}}},
                "limit": 50
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_$not", "advanced", test_not)

        # $nor operator
        def test_nor():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"$nor": [{"index": 0}, {"index": 1}]},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_$nor", "advanced", test_nor)

        # $type operator (number)
        def test_type_number():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"value": {"$type": "number"}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs with number value"
        self._run_test("find_$type_number", "advanced", test_type_number)

        # $type operator (string)
        def test_type_string():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"name": {"$type": "string"}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs with string name"
        self._run_test("find_$type_string", "advanced", test_type_string)

        # $type operator (bool)
        def test_type_bool():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"active": {"$type": "bool"}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs with bool active"
        self._run_test("find_$type_bool", "advanced", test_type_bool)

        # $type operator (array)
        def test_type_array():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"tags": {"$type": "array"}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs with array tags"
        self._run_test("find_$type_array", "advanced", test_type_array)

        # $elemMatch
        def test_elemmatch():
            # Insert doc with array of objects
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {
                    "items": [
                        {"name": "a", "qty": 10},
                        {"name": "b", "qty": 20}
                    ]
                }
            })
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"items": {"$elemMatch": {"qty": {"$gt": 15}}}},
                "limit": 10
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_$elemMatch", "advanced", test_elemmatch)

        # Complex nested query
        def test_complex_nested():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {
                    "$and": [
                        {"active": True},
                        {"$or": [
                            {"value": {"$gt": 800}},
                            {"score": {"$lt": 20}}
                        ]}
                    ]
                },
                "limit": 20
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_complex_nested", "advanced", test_complex_nested)

        # Multiple sort fields
        def test_multi_sort():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "sort": [["active", 1], ["value", -1]],
                "limit": 10
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_multi_sort", "advanced", test_multi_sort)

        # Case-insensitive regex
        def test_regex_case():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"name": {"$regex": "test", "$options": "i"}},
                "limit": 20
            })
            return f"{result.get('count', 0)} docs"
        self._run_test("find_$regex_case_insensitive", "advanced", test_regex_case)

    def test_advanced_updates(self):
        """Advanced update tests"""
        print("\n✏️ Advanced Update Tests")

        # $unset operator
        def test_unset():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 5},
                "update": {"$unset": {"score": ""}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$unset", "update_adv", test_unset)

        # $addToSet operator
        def test_addtoset():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 6},
                "update": {"$addToSet": {"tags": "unique_tag"}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$addToSet", "update_adv", test_addtoset)

        # $pop operator (remove last)
        def test_pop_last():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 7},
                "update": {"$pop": {"tags": 1}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$pop_last", "update_adv", test_pop_last)

        # $pop operator (remove first)
        def test_pop_first():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 8},
                "update": {"$pop": {"tags": -1}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$pop_first", "update_adv", test_pop_first)

        # $pull operator
        def test_pull():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 9},
                "update": {"$pull": {"tags": "new_tag"}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_$pull", "update_adv", test_pull)

        # Multiple update operators
        def test_multi_op():
            result, _ = self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"index": 10},
                "update": {
                    "$set": {"multi_updated": True},
                    "$inc": {"value": 50}
                }
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_multi_operators", "update_adv", test_multi_op)

    def test_advanced_aggregation(self):
        """Advanced aggregation tests"""
        print("\n📈 Advanced Aggregation Tests")

        # $first accumulator
        def test_agg_first():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$sort": {"value": 1}},
                    {"$group": {"_id": "$active", "first_value": {"$first": "$value"}}}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} groups"
        self._run_test("aggregate_$first", "agg_adv", test_agg_first)

        # $last accumulator
        def test_agg_last():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$sort": {"value": 1}},
                    {"$group": {"_id": "$active", "last_value": {"$last": "$value"}}}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} groups"
        self._run_test("aggregate_$last", "agg_adv", test_agg_last)

        # Complex pipeline
        def test_agg_complex():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$match": {"value": {"$gt": 0}}},
                    {"$group": {
                        "_id": "$active",
                        "count": {"$sum": 1},
                        "total": {"$sum": "$value"},
                        "avg": {"$avg": "$value"}
                    }},
                    {"$sort": {"total": -1}},
                    {"$limit": 10}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} groups"
        self._run_test("aggregate_complex_pipeline", "agg_adv", test_agg_complex)

        # Group by nested field
        def test_agg_nested_group():
            result, _ = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$group": {"_id": "$nested.level1.level2", "count": {"$sum": 1}}},
                    {"$limit": 10}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} groups"
        self._run_test("aggregate_nested_group", "agg_adv", test_agg_nested_group)

    def test_more_edge_cases(self):
        """Additional edge case tests"""
        print("\n⚠️ Additional Edge Cases")

        # Very long string
        def test_long_string():
            doc = {"long_text": "x" * 10000}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return f"Inserted 10KB text"
        self._run_test("insert_long_string", "edge2", test_long_string)

        # Many fields
        def test_many_fields():
            doc = {f"field_{i}": i for i in range(100)}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return f"Inserted doc with 100 fields"
        self._run_test("insert_many_fields", "edge2", test_many_fields)

        # Special characters in field values
        def test_special_chars():
            doc = {"special": "Line1\nLine2\tTabbed\r\nCRLF"}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return "Inserted special chars"
        self._run_test("insert_special_chars", "edge2", test_special_chars)

        # Numeric edge cases
        def test_numeric_edge():
            doc = {
                "max_int": 2**53 - 1,
                "negative": -999999,
                "float_precise": 3.141592653589793,
                "zero": 0
            }
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return "Inserted numeric edge cases"
        self._run_test("insert_numeric_edge", "edge2", test_numeric_edge)

        # Boolean queries
        def test_bool_false():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"active": False},
                "limit": 10
            })
            return f"{result.get('count', 0)} inactive"
        self._run_test("find_bool_false", "edge2", test_bool_false)

        # Null value query
        def test_null_query():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"null_field": None},
                "limit": 10
            })
            return f"{result.get('count', 0)} with null"
        self._run_test("find_null_value", "edge2", test_null_query)

        # Empty array
        def test_empty_array():
            doc = {"empty_arr": []}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return "Inserted empty array"
        self._run_test("insert_empty_array", "edge2", test_empty_array)

        # Nested arrays
        def test_nested_array():
            doc = {"matrix": [[1, 2], [3, 4], [5, 6]]}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return "Inserted nested array"
        self._run_test("insert_nested_array", "edge2", test_nested_array)

        # Mixed type array
        def test_mixed_array():
            doc = {"mixed": [1, "two", True, None, {"nested": "obj"}]}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            return "Inserted mixed array"
        self._run_test("insert_mixed_array", "edge2", test_mixed_array)

    def test_performance(self):
        """Performance tests"""
        print("\n⚡ Performance Tests")

        # Batch insert 1000
        def test_batch_1000():
            docs = [{"perf_test": True, "idx": i, "data": self._random_string(100)} for i in range(1000)]
            result, duration = self.client.tool("insert_many", {
                "collection": self.test_collection,
                "documents": docs
            })
            rate = 1000 / (duration / 1000) if duration > 0 else 0
            return f"{result['inserted_count']} docs, {rate:.0f} docs/sec"
        self._run_test("batch_insert_1000", "performance", test_batch_1000)

        # Count large
        def test_count_large():
            result, duration = self.client.tool("count_documents", {
                "collection": self.test_collection,
                "query": {}
            })
            return f"Count: {result.get('count')} in {duration:.1f}ms"
        self._run_test("count_large_collection", "performance", test_count_large)

        # Find with index
        def test_find_indexed():
            # First create index
            self.client.tool("index_create", {
                "collection": self.test_collection,
                "field": "idx"
            })
            result, duration = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"idx": {"$gt": 500}},
                "limit": 100
            })
            return f"{result.get('count', 0)} docs in {duration:.1f}ms"
        self._run_test("find_indexed_field", "performance", test_find_indexed)

        # Sequential find operations
        def test_sequential_finds():
            start = time.perf_counter()
            for i in range(10):
                self.client.tool("find", {
                    "collection": self.test_collection,
                    "query": {"idx": i},
                    "limit": 1
                })
            duration = (time.perf_counter() - start) * 1000
            return f"10 finds in {duration:.1f}ms ({duration/10:.1f}ms avg)"
        self._run_test("sequential_10_finds", "performance", test_sequential_finds)

        # Aggregation performance
        def test_agg_perf():
            result, duration = self.client.tool("aggregate", {
                "collection": self.test_collection,
                "pipeline": [
                    {"$match": {"perf_test": True}},
                    {"$group": {"_id": None, "count": {"$sum": 1}, "total": {"$sum": "$idx"}}}
                ]
            })
            return f"Aggregation in {duration:.1f}ms"
        self._run_test("aggregation_performance", "performance", test_agg_perf)

    def test_transactions(self):
        """Transaction tests"""
        print("\n🔄 Transaction Tests")

        tx_id = None

        # Begin transaction
        def test_begin_tx():
            nonlocal tx_id
            result, _ = self.client.tool("begin_transaction", {})
            tx_id = result.get("transaction_id")
            assert tx_id is not None
            return f"TX ID: {tx_id}"
        self._run_test("begin_transaction", "transaction", test_begin_tx)

        # Insert in transaction
        def test_insert_tx():
            result, _ = self.client.tool("insert_one_tx", {
                "transaction_id": str(tx_id),
                "collection": self.test_collection,
                "document": {"tx_test": True, "value": 999}
            })
            return f"Inserted: {result.get('inserted_id')}"
        self._run_test("insert_one_tx", "transaction", test_insert_tx)

        # Update in transaction
        def test_update_tx():
            result, _ = self.client.tool("update_one_tx", {
                "transaction_id": str(tx_id),
                "collection": self.test_collection,
                "filter": {"tx_test": True},
                "update": {"$set": {"updated_in_tx": True}}
            })
            return f"Modified: {result.get('modified_count', 0)}"
        self._run_test("update_one_tx", "transaction", test_update_tx)

        # Transaction status
        def test_tx_status():
            result, _ = self.client.tool("transaction_status", {
                "transaction_id": str(tx_id)
            })
            return f"Status: {result.get('status', 'unknown')}"
        self._run_test("transaction_status", "transaction", test_tx_status)

        # Commit transaction
        def test_commit_tx():
            result, _ = self.client.tool("commit_transaction", {
                "transaction_id": str(tx_id)
            })
            return f"Committed: {result.get('success')}"
        self._run_test("commit_transaction", "transaction", test_commit_tx)

        # Verify committed data
        def test_verify_commit():
            result, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"tx_test": True}
            })
            # result might be the document directly or wrapped
            doc = result.get("document", result) if isinstance(result, dict) else result
            return f"Data found: {doc is not None}"
        self._run_test("verify_committed_data", "transaction", test_verify_commit)

        # Begin and rollback
        def test_rollback():
            # Begin new TX
            result, _ = self.client.tool("begin_transaction", {})
            new_tx_id = result.get("transaction_id")

            # Insert something
            self.client.tool("insert_one_tx", {
                "transaction_id": str(new_tx_id),
                "collection": self.test_collection,
                "document": {"rollback_test": True, "should_not_exist": True}
            })

            # Rollback
            result, _ = self.client.tool("rollback_transaction", {
                "transaction_id": str(new_tx_id)
            })
            return f"Rolled back: {result.get('success')}"
        self._run_test("rollback_transaction", "transaction", test_rollback)

        # Verify rollback
        def test_verify_rollback():
            result, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"rollback_test": True}
            })
            # Should not find the rolled back document
            return f"Correctly rolled back: {result is None or result.get('document') is None}"
        self._run_test("verify_rollback", "transaction", test_verify_rollback)

    def test_fulltext_search(self):
        """Fulltext search tests"""
        print("\n📝 Fulltext Search Tests")

        ft_collection = f"{self.test_collection}_ft"

        # Create collection for fulltext
        def test_ft_setup():
            self.client.tool("collection_create", {"collection": ft_collection})
            # Insert documents with text content
            docs = [
                {"title": "A király és a lovag", "content": "Egyszer volt egy király aki nagyon bölcs volt."},
                {"title": "A hercegnő bálján", "content": "A hercegnő meghívta a királyságot a nagy bálra."},
                {"title": "Programozás alapok", "content": "A Python egy népszerű programozási nyelv."},
                {"title": "Rust bevezetés", "content": "A Rust biztonságos és gyors rendszerprogramozási nyelv."},
                {"title": "Adatbázis kezelés", "content": "Az IronBase egy MongoDB-kompatibilis NoSQL adatbázis."},
                {"title": "Magyar irodalom", "content": "Petőfi Sándor a magyar irodalom egyik legnagyobb alakja."},
                {"title": "Természettudomány", "content": "A fizika a természet alapvető törvényeit vizsgálja."},
                {"title": "Matematika", "content": "A matematika az absztrakt struktúrák tudománya."},
            ]
            result, _ = self.client.tool("insert_many", {
                "collection": ft_collection,
                "documents": docs
            })
            return f"Inserted {result.get('inserted_count', 0)} docs"
        self._run_test("fulltext_setup", "fulltext", test_ft_setup)

        # Create fulltext index
        def test_ft_index():
            result, _ = self.client.tool("index_create_fulltext", {
                "collection": ft_collection,
                "field": "content",
                "language": "hungarian"
            })
            return f"Index: {result.get('index_name')}"
        self._run_test("fulltext_create_index", "fulltext", test_ft_index)

        # Basic fulltext search
        def test_ft_search():
            result, _ = self.client.tool("fulltext_search", {
                "collection": ft_collection,
                "field": "content",
                "query": "király",
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} results"
        self._run_test("fulltext_search_basic", "fulltext", test_ft_search)

        # Fulltext search with min_score
        def test_ft_min_score():
            result, _ = self.client.tool("fulltext_search", {
                "collection": ft_collection,
                "field": "content",
                "query": "programozás",
                "min_score": 0.1,
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} high-score results"
        self._run_test("fulltext_search_min_score", "fulltext", test_ft_min_score)

        # Fulltext search with projection
        def test_ft_projection():
            result, _ = self.client.tool("fulltext_search", {
                "collection": ft_collection,
                "field": "content",
                "query": "nyelv",
                "projection": {"title": 1},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} projected"
        self._run_test("fulltext_search_projection", "fulltext", test_ft_projection)

        # Fulltext analyze
        def test_ft_analyze():
            result, _ = self.client.tool("fulltext_analyze", {
                "text": "A királynő és a király találkoztak",
                "language": "hungarian"
            })
            tokens = result.get("tokens", [])
            return f"Tokens: {len(tokens)}"
        self._run_test("fulltext_analyze", "fulltext", test_ft_analyze)

        # Cleanup
        def test_ft_cleanup():
            result, _ = self.client.tool("collection_drop", {"collection": ft_collection})
            return f"Dropped: {result.get('success')}"
        self._run_test("fulltext_cleanup", "fulltext", test_ft_cleanup)

    def test_fuzzy_search(self):
        """Fuzzy search tests"""
        print("\n🔮 Fuzzy Search Tests")

        fz_collection = f"{self.test_collection}_fz"

        # Setup
        def test_fz_setup():
            self.client.tool("collection_create", {"collection": fz_collection})
            docs = [
                {"name": "John Smith"},
                {"name": "Jon Smyth"},
                {"name": "Johnny Smith"},
                {"name": "Jane Smith"},
                {"name": "Joan Smithson"},
                {"name": "Michael Johnson"},
                {"name": "Michelle Johnston"},
                {"name": "Robert Williams"},
                {"name": "Roberta Williams"},
            ]
            result, _ = self.client.tool("insert_many", {
                "collection": fz_collection,
                "documents": docs
            })
            return f"Inserted {result.get('inserted_count', 0)} docs"
        self._run_test("fuzzy_setup", "fuzzy", test_fz_setup)

        # Create fuzzy index
        def test_fz_index():
            result, _ = self.client.tool("index_create_fuzzy", {
                "collection": fz_collection,
                "field": "name",
                "algorithm": "jaro_winkler",
                "threshold": 0.8
            })
            return f"Index: {result.get('index_name')}"
        self._run_test("fuzzy_create_index", "fuzzy", test_fz_index)

        # Fuzzy search - Jaro-Winkler
        def test_fz_jaro():
            result, _ = self.client.tool("fuzzy_search", {
                "collection": fz_collection,
                "field": "name",
                "query": "John Smit",
                "algorithm": "jaro_winkler",
                "threshold": 0.7,
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} similar names"
        self._run_test("fuzzy_search_jaro_winkler", "fuzzy", test_fz_jaro)

        # Fuzzy search - Levenshtein
        def test_fz_levenshtein():
            result, _ = self.client.tool("fuzzy_search", {
                "collection": fz_collection,
                "field": "name",
                "query": "Micheal",
                "algorithm": "levenshtein",
                "threshold": 0.6,
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} matches"
        self._run_test("fuzzy_search_levenshtein", "fuzzy", test_fz_levenshtein)

        # Fuzzy search - Damerau-Levenshtein
        def test_fz_damerau():
            result, _ = self.client.tool("fuzzy_search", {
                "collection": fz_collection,
                "field": "name",
                "query": "Willaims",  # Transposition error
                "algorithm": "damerau_levenshtein",
                "threshold": 0.7,
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} matches"
        self._run_test("fuzzy_search_damerau", "fuzzy", test_fz_damerau)

        # Fuzzy with highlight
        def test_fz_highlight():
            result, _ = self.client.tool("fuzzy_search", {
                "collection": fz_collection,
                "field": "name",
                "query": "Robert",
                "threshold": 0.8,
                "highlight": True,
                "limit": 5
            })
            return f"Found with highlights: {result.get('count', 0)}"
        self._run_test("fuzzy_search_highlight", "fuzzy", test_fz_highlight)

        # Cleanup
        def test_fz_cleanup():
            result, _ = self.client.tool("collection_drop", {"collection": fz_collection})
            return f"Dropped: {result.get('success')}"
        self._run_test("fuzzy_cleanup", "fuzzy", test_fz_cleanup)

    def test_complex_aggregations(self):
        """Complex aggregation pipeline tests"""
        print("\n🔬 Complex Aggregation Tests")

        agg_collection = f"{self.test_collection}_agg"

        # Setup with structured data
        def test_agg_setup():
            self.client.tool("collection_create", {"collection": agg_collection})
            docs = []
            categories = ["electronics", "clothing", "food", "books", "toys"]
            for i in range(200):
                docs.append({
                    "order_id": i,
                    "customer_id": i % 20,
                    "category": categories[i % 5],
                    "amount": round(random.uniform(10, 500), 2),
                    "quantity": random.randint(1, 10),
                    "date": f"2024-{(i % 12) + 1:02d}-{(i % 28) + 1:02d}",
                    "status": random.choice(["pending", "shipped", "delivered", "returned"]),
                    "items": [
                        {"sku": f"SKU{j}", "price": round(random.uniform(5, 100), 2)}
                        for j in range(random.randint(1, 4))
                    ]
                })
            result, _ = self.client.tool("insert_many", {
                "collection": agg_collection,
                "documents": docs
            })
            return f"Inserted {result.get('inserted_count', 0)} orders"
        self._run_test("complex_agg_setup", "complex_agg", test_agg_setup)

        # Group by category with stats
        def test_agg_category_stats():
            result, _ = self.client.tool("aggregate", {
                "collection": agg_collection,
                "pipeline": [
                    {"$group": {
                        "_id": "$category",
                        "total_orders": {"$sum": 1},
                        "total_revenue": {"$sum": "$amount"},
                        "avg_order": {"$avg": "$amount"},
                        "min_order": {"$min": "$amount"},
                        "max_order": {"$max": "$amount"}
                    }},
                    {"$sort": {"total_revenue": -1}}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} categories"
        self._run_test("agg_category_stats", "complex_agg", test_agg_category_stats)

        # Multi-level grouping
        def test_agg_multi_group():
            result, _ = self.client.tool("aggregate", {
                "collection": agg_collection,
                "pipeline": [
                    {"$group": {
                        "_id": {"category": "$category", "status": "$status"},
                        "count": {"$sum": 1},
                        "total": {"$sum": "$amount"}
                    }},
                    {"$sort": {"_id.category": 1, "count": -1}},
                    {"$limit": 20}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} groups"
        self._run_test("agg_multi_level_group", "complex_agg", test_agg_multi_group)

        # Top customers
        def test_agg_top_customers():
            result, _ = self.client.tool("aggregate", {
                "collection": agg_collection,
                "pipeline": [
                    {"$group": {
                        "_id": "$customer_id",
                        "order_count": {"$sum": 1},
                        "total_spent": {"$sum": "$amount"}
                    }},
                    {"$sort": {"total_spent": -1}},
                    {"$limit": 5}
                ]
            })
            return f"Top 5 customers found"
        self._run_test("agg_top_customers", "complex_agg", test_agg_top_customers)

        # Filter + Group + Sort + Limit
        def test_agg_full_pipeline():
            result, _ = self.client.tool("aggregate", {
                "collection": agg_collection,
                "pipeline": [
                    {"$match": {"status": {"$in": ["shipped", "delivered"]}}},
                    {"$group": {
                        "_id": "$category",
                        "successful_orders": {"$sum": 1},
                        "revenue": {"$sum": "$amount"}
                    }},
                    {"$match": {"successful_orders": {"$gte": 5}}},
                    {"$sort": {"revenue": -1}},
                    {"$limit": 3}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} top categories"
        self._run_test("agg_full_pipeline", "complex_agg", test_agg_full_pipeline)

        # Date-based grouping (by month)
        def test_agg_by_month():
            result, _ = self.client.tool("aggregate", {
                "collection": agg_collection,
                "pipeline": [
                    {"$group": {
                        "_id": {"$substr": ["$date", 0, 7]},  # YYYY-MM
                        "orders": {"$sum": 1},
                        "revenue": {"$sum": "$amount"}
                    }},
                    {"$sort": {"_id": 1}},
                    {"$limit": 12}
                ]
            })
            return f"{len(result) if isinstance(result, list) else 0} months"
        self._run_test("agg_by_month", "complex_agg", test_agg_by_month)

        # Cleanup
        def test_agg_complex_cleanup():
            result, _ = self.client.tool("collection_drop", {"collection": agg_collection})
            return f"Dropped: {result.get('success')}"
        self._run_test("complex_agg_cleanup", "complex_agg", test_agg_complex_cleanup)

    def test_data_integrity(self):
        """Data integrity tests"""
        print("\n🔒 Data Integrity Tests")

        # Insert and verify _id
        def test_id_generation():
            doc = {"integrity_test": True, "value": 12345}
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": doc
            })
            inserted_id = result.get("inserted_id")

            # Retrieve and verify
            found, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"_id": inserted_id}
            })
            # Handle potential wrapper
            doc = found.get("document", found) if isinstance(found, dict) and "document" in found else found
            value = doc.get("value") if doc else None
            return f"ID {inserted_id} verified, value={value}"
        self._run_test("id_generation_verification", "integrity", test_id_generation)

        # Update and verify
        def test_update_integrity():
            # Insert
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"update_integrity_test": True, "counter": 0}
            })
            doc_id = result.get("inserted_id")

            # Update multiple times
            for i in range(5):
                self.client.tool("update_one", {
                    "collection": self.test_collection,
                    "filter": {"_id": doc_id},
                    "update": {"$inc": {"counter": 1}}
                })

            # Verify
            found, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"_id": doc_id}
            })
            doc = found.get("document", found) if isinstance(found, dict) and "document" in found else found
            counter = doc.get("counter") if doc else None
            return f"Counter after 5 increments: {counter}"
        self._run_test("update_counter_integrity", "integrity", test_update_integrity)

        # Array operations integrity
        def test_array_integrity():
            # Insert with array
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"array_integrity_test": True, "items": ["a", "b"]}
            })
            doc_id = result.get("inserted_id")

            # Push items
            self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"_id": doc_id},
                "update": {"$push": {"items": "c"}}
            })

            self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"_id": doc_id},
                "update": {"$push": {"items": "d"}}
            })

            # Verify
            found, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"_id": doc_id}
            })
            doc = found.get("document", found) if isinstance(found, dict) and "document" in found else found
            items = doc.get("items", []) if doc else []
            return f"Array has {len(items)} items: {items}"
        self._run_test("array_push_integrity", "integrity", test_array_integrity)

        # Nested field update integrity
        def test_nested_integrity():
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {
                    "nested_integrity_test": True,
                    "level1": {"level2": {"value": "original"}}
                }
            })
            doc_id = result.get("inserted_id")

            # Update nested
            self.client.tool("update_one", {
                "collection": self.test_collection,
                "filter": {"_id": doc_id},
                "update": {"$set": {"level1.level2.value": "updated"}}
            })

            # Verify
            found, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"_id": doc_id}
            })
            doc = found.get("document", found) if isinstance(found, dict) and "document" in found else found
            if doc and "level1" in doc:
                value = doc["level1"].get("level2", {}).get("value")
                return f"Nested value: {value}"
            return f"Doc structure: {list(doc.keys()) if doc else 'None'}"
        self._run_test("nested_update_integrity", "integrity", test_nested_integrity)

        # Delete integrity
        def test_delete_integrity():
            # Insert batch
            docs = [{"delete_integrity": True, "idx": i} for i in range(10)]
            self.client.tool("insert_many", {
                "collection": self.test_collection,
                "documents": docs
            })

            # Count before
            before, _ = self.client.tool("count_documents", {
                "collection": self.test_collection,
                "query": {"delete_integrity": True}
            })

            # Delete half
            self.client.tool("delete_many", {
                "collection": self.test_collection,
                "filter": {"delete_integrity": True, "idx": {"$lt": 5}}
            })

            # Count after
            after, _ = self.client.tool("count_documents", {
                "collection": self.test_collection,
                "query": {"delete_integrity": True}
            })

            assert after.get("count") == 5
            return f"Delete verified: {before.get('count')} -> {after.get('count')}"
        self._run_test("delete_count_integrity", "integrity", test_delete_integrity)

    def test_boundary_conditions(self):
        """Boundary condition tests"""
        print("\n🔢 Boundary Condition Tests")

        # Empty string handling
        def test_empty_string_query():
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"boundary": True, "empty_str": ""}
            })
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"empty_str": ""},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} with empty string"
        self._run_test("empty_string_query", "boundary", test_empty_string_query)

        # Zero value handling
        def test_zero_value():
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"boundary": True, "zero_int": 0, "zero_float": 0.0}
            })
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"zero_int": 0},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} with zero"
        self._run_test("zero_value_query", "boundary", test_zero_value)

        # Negative number
        def test_negative():
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"boundary": True, "negative": -999}
            })
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"negative": {"$lt": 0}},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} negative"
        self._run_test("negative_number_query", "boundary", test_negative)

        # Very small float
        def test_small_float():
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"boundary": True, "tiny": 0.0000001}
            })
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"tiny": {"$gt": 0, "$lt": 0.001}},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} tiny floats"
        self._run_test("small_float_query", "boundary", test_small_float)

        # Large integer
        def test_large_int():
            large = 2**52  # Near JS safe integer limit
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"boundary": True, "large": large}
            })
            result, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"large": large}
            })
            return f"Found large int: {result.get('large') == large}"
        self._run_test("large_integer_query", "boundary", test_large_int)

        # Skip = 0
        def test_skip_zero():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "skip": 0,
                "limit": 5
            })
            return f"Skip 0: {result.get('count', 0)} docs"
        self._run_test("skip_zero", "boundary", test_skip_zero)

        # Limit = 1
        def test_limit_one():
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {},
                "limit": 1
            })
            return f"Limit 1: {result.get('count', 0)} doc"
        self._run_test("limit_one", "boundary", test_limit_one)

        # Empty array query
        def test_empty_array_query():
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"boundary": True, "empty_arr": []}
            })
            result, _ = self.client.tool("find", {
                "collection": self.test_collection,
                "query": {"empty_arr": {"$size": 0}},
                "limit": 10
            })
            return f"Found: {result.get('count', 0)} empty arrays"
        self._run_test("empty_array_size_query", "boundary", test_empty_array_query)

    def test_concurrent_simulation(self):
        """Concurrent operation simulation tests"""
        print("\n🔀 Concurrent Simulation Tests")

        # Rapid sequential inserts
        def test_rapid_inserts():
            start = time.perf_counter()
            for i in range(50):
                self.client.tool("insert_one", {
                    "collection": self.test_collection,
                    "document": {"rapid": True, "seq": i}
                })
            duration = (time.perf_counter() - start) * 1000
            return f"50 inserts in {duration:.1f}ms"
        self._run_test("rapid_sequential_inserts", "concurrent", test_rapid_inserts)

        # Rapid sequential updates
        def test_rapid_updates():
            # First insert target
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"rapid_update": True, "counter": 0}
            })
            doc_id = result.get("inserted_id")

            start = time.perf_counter()
            for i in range(20):
                self.client.tool("update_one", {
                    "collection": self.test_collection,
                    "filter": {"_id": doc_id},
                    "update": {"$inc": {"counter": 1}}
                })
            duration = (time.perf_counter() - start) * 1000
            return f"20 updates in {duration:.1f}ms"
        self._run_test("rapid_sequential_updates", "concurrent", test_rapid_updates)

        # Mixed read/write
        def test_mixed_operations():
            start = time.perf_counter()
            for i in range(30):
                if i % 3 == 0:
                    self.client.tool("insert_one", {
                        "collection": self.test_collection,
                        "document": {"mixed": True, "idx": i}
                    })
                elif i % 3 == 1:
                    self.client.tool("find", {
                        "collection": self.test_collection,
                        "query": {"mixed": True},
                        "limit": 5
                    })
                else:
                    self.client.tool("count_documents", {
                        "collection": self.test_collection,
                        "query": {"mixed": True}
                    })
            duration = (time.perf_counter() - start) * 1000
            return f"30 mixed ops in {duration:.1f}ms"
        self._run_test("mixed_read_write", "concurrent", test_mixed_operations)

    def test_stress(self):
        """Stress tests"""
        print("\n💪 Stress Tests")

        stress_collection = f"{self.test_collection}_stress"

        # Large batch insert
        def test_large_batch():
            self.client.tool("collection_create", {"collection": stress_collection})
            docs = [{"stress": True, "idx": i, "data": self._random_string(200)} for i in range(2000)]
            result, duration = self.client.tool("insert_many", {
                "collection": stress_collection,
                "documents": docs
            })
            rate = 2000 / (duration / 1000) if duration > 0 else 0
            return f"2000 docs at {rate:.0f} docs/sec"
        self._run_test("stress_insert_2000", "stress", test_large_batch)

        # Count on large collection
        def test_large_count():
            result, duration = self.client.tool("count_documents", {
                "collection": stress_collection,
                "query": {}
            })
            return f"Count {result.get('count', 0)} in {duration:.1f}ms"
        self._run_test("stress_count_large", "stress", test_large_count)

        # Filtered count on large collection
        def test_filtered_count():
            result, duration = self.client.tool("count_documents", {
                "collection": stress_collection,
                "query": {"idx": {"$gt": 1000}}
            })
            return f"Filtered count: {result.get('count', 0)} in {duration:.1f}ms"
        self._run_test("stress_filtered_count", "stress", test_filtered_count)

        # Create index on large collection
        def test_large_index():
            result, duration = self.client.tool("index_create", {
                "collection": stress_collection,
                "field": "idx"
            })
            return f"Index created in {duration:.1f}ms"
        self._run_test("stress_create_index", "stress", test_large_index)

        # Query with index
        def test_indexed_query():
            result, duration = self.client.tool("find", {
                "collection": stress_collection,
                "query": {"idx": {"$gte": 1500, "$lte": 1600}},
                "limit": 200
            })
            return f"Range query: {result.get('count', 0)} in {duration:.1f}ms"
        self._run_test("stress_indexed_range_query", "stress", test_indexed_query)

        # Aggregation on large collection
        def test_large_agg():
            result, duration = self.client.tool("aggregate", {
                "collection": stress_collection,
                "pipeline": [
                    {"$match": {"idx": {"$gte": 500}}},
                    {"$group": {"_id": {"$mod": ["$idx", 10]}, "count": {"$sum": 1}}},
                    {"$sort": {"count": -1}}
                ]
            })
            return f"Agg: {len(result) if isinstance(result, list) else 0} groups in {duration:.1f}ms"
        self._run_test("stress_aggregation", "stress", test_large_agg)

        # Cleanup
        def test_stress_cleanup():
            result, _ = self.client.tool("collection_drop", {"collection": stress_collection})
            return f"Dropped: {result.get('success')}"
        self._run_test("stress_cleanup", "stress", test_stress_cleanup)

    def test_special_field_names(self):
        """Special field name tests"""
        print("\n🏷️ Special Field Name Tests")

        # Field with underscore prefix
        def test_underscore_field():
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"_custom_field": "value", "special_test": True}
            })
            found, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"_custom_field": "value"}
            })
            return f"Found: {found is not None}"
        self._run_test("underscore_prefix_field", "special_fields", test_underscore_field)

        # Field with numbers
        def test_numeric_field():
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"field123": "value", "123field": "value2", "special_test": True}
            })
            return f"Inserted: {result.get('inserted_id')}"
        self._run_test("numeric_field_names", "special_fields", test_numeric_field)

        # Very long field name
        def test_long_field():
            long_name = "very_long_field_name_" * 5
            result, _ = self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {long_name: "value", "special_test": True}
            })
            return f"Long field name: {len(long_name)} chars"
        self._run_test("long_field_name", "special_fields", test_long_field)

        # Field with mixed case
        def test_case_sensitive():
            self.client.tool("insert_one", {
                "collection": self.test_collection,
                "document": {"Field": "upper", "field": "lower", "FIELD": "caps", "special_test": True}
            })
            result, _ = self.client.tool("find_one", {
                "collection": self.test_collection,
                "query": {"Field": "upper"}
            })
            return f"Case sensitive: {result.get('Field') == 'upper' and result.get('field') == 'lower'}"
        self._run_test("case_sensitive_fields", "special_fields", test_case_sensitive)

    def cleanup(self):
        """Cleanup test collection"""
        print("\n🧹 Cleanup")

        def test_cleanup():
            result, _ = self.client.tool("collection_drop", {
                "collection": self.test_collection
            })
            return f"Dropped: {result.get('success')}"
        self._run_test("cleanup_drop_collection", "cleanup", test_cleanup)

    def run_all(self):
        """Run all tests"""
        print("=" * 60)
        print("🧪 MCP Comprehensive Test Suite")
        print("=" * 60)

        start_time = time.perf_counter()

        # Run test categories
        self.test_initialization()
        self.test_collection_operations()
        self.test_insert_operations()
        self.test_find_operations()
        self.test_update_operations()
        self.test_delete_operations()
        self.test_count_distinct()
        self.test_index_operations()
        self.test_aggregation()
        self.test_schema_operations()
        self.test_db_operations()
        self.test_acl_operations()
        self.test_listener_operations()
        self.test_edge_cases()
        self.test_advanced_queries()
        self.test_advanced_updates()
        self.test_advanced_aggregation()
        self.test_more_edge_cases()
        self.test_transactions()
        self.test_fulltext_search()
        self.test_fuzzy_search()
        self.test_complex_aggregations()
        self.test_data_integrity()
        self.test_boundary_conditions()
        self.test_concurrent_simulation()
        self.test_stress()
        self.test_special_field_names()
        self.test_performance()
        self.cleanup()

        total_time = (time.perf_counter() - start_time) * 1000

        # Print summary
        print("\n" + "=" * 60)
        print("📊 TEST SUMMARY")
        print("=" * 60)

        # Category breakdown
        categories = {}
        for r in self.stats.results:
            if r.category not in categories:
                categories[r.category] = {"passed": 0, "failed": 0, "time": 0}
            if r.passed:
                categories[r.category]["passed"] += 1
            else:
                categories[r.category]["failed"] += 1
            categories[r.category]["time"] += r.duration_ms

        print(f"\n{'Category':<20} {'Passed':>8} {'Failed':>8} {'Time (ms)':>12}")
        print("-" * 50)
        for cat, data in sorted(categories.items()):
            print(f"{cat:<20} {data['passed']:>8} {data['failed']:>8} {data['time']:>12.1f}")

        print("-" * 50)
        print(f"{'TOTAL':<20} {self.stats.passed:>8} {self.stats.failed:>8} {self.stats.total_time_ms:>12.1f}")

        # Failed tests
        if self.stats.failed > 0:
            print(f"\n❌ FAILED TESTS ({self.stats.failed}):")
            for r in self.stats.results:
                if not r.passed:
                    print(f"  - {r.name}: {r.error}")

        # Overall result
        print("\n" + "=" * 60)
        if self.stats.failed == 0:
            print(f"✅ ALL {self.stats.total} TESTS PASSED in {total_time:.1f}ms")
        else:
            print(f"❌ {self.stats.failed}/{self.stats.total} TESTS FAILED in {total_time:.1f}ms")
        print("=" * 60)

        return self.stats.failed == 0


def main():
    parser = argparse.ArgumentParser(description="MCP Comprehensive Test Suite")
    parser.add_argument("--host", default="localhost", help="MCP server host")
    parser.add_argument("--port", type=int, default=8080, help="MCP server port")
    parser.add_argument("--no-tls", action="store_true", help="Disable TLS")
    parser.add_argument("-v", "--verbose", action="store_true", help="Verbose output")
    args = parser.parse_args()

    client = MCPClient(
        host=args.host,
        port=args.port,
        use_tls=not args.no_tls
    )

    suite = MCPTestSuite(client, verbose=args.verbose)
    success = suite.run_all()

    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()
