#!/usr/bin/env python3
"""Simple tools/list schema sanity checks.

Heuristic parser for mcp-server/src/tools/mod.rs that verifies:
- tools listed match dispatch names (extra/missing)
- each tool has inputSchema/properties
- required fields are defined in properties
- optional policy checks for known tools
"""

from __future__ import annotations

import re
from pathlib import Path

TOOLS_PATH = Path(__file__).resolve().parents[1] / "src" / "tools" / "mod.rs"

OPTIONAL_QUERY_TOOLS = {"find", "find_one", "find_with_hint", "explain"}
SORT_OBJECT_TOOLS = {"find", "find_with_hint"}
LIMIT_FIELDS = {"limit", "skip", "max_operations", "min_word_length"}


def parse_dispatch_names(text: str) -> set[str]:
    start = text.find("fn dispatch_tool_inner")
    end = text.find("fn get_all_tools_json")
    if start == -1 or end == -1 or end <= start:
        return set()
    block = text[start:end]
    return set(re.findall(r'"([a-z0-9_]+)"', block))


def parse_tool_blocks(text: str) -> list[dict]:
    tools_section = text.split('"tools": [', 1)
    if len(tools_section) < 2:
        return []

    lines = tools_section[1].splitlines()
    blocks: list[dict] = []
    current = None

    for line in lines:
        if '"name":' in line:
            if current:
                blocks.append(current)
            current = {"lines": [line]}
        elif current:
            current["lines"].append(line)

    if current:
        blocks.append(current)

    return blocks


def extract_name(block: dict) -> str | None:
    for line in block["lines"]:
        m = re.search(r'"name"\s*:\s*"([^"]+)"', line)
        if m:
            return m.group(1)
    return None


def extract_required(block_text: str) -> list[str]:
    m = re.search(r'"required"\s*:\s*\[(.*?)\]', block_text, re.S)
    if not m:
        return []
    return re.findall(r'"([^"]+)"', m.group(1))


def extract_properties(block_text: str) -> tuple[bool, set[str]]:
    start = block_text.find('"properties"')
    if start == -1:
        return False, set()
    brace_start = block_text.find("{", start)
    if brace_start == -1:
        return False, set()

    depth = 0
    keys = set()
    in_string = False
    escape = False
    key_buffer = []
    capturing_key = False
    i = brace_start
    while i < len(block_text):
        ch = block_text[i]
        if in_string:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_string = False
                if depth == 1 and capturing_key:
                    key = "".join(key_buffer)
                    if key:
                        keys.add(key)
                    key_buffer = []
                    capturing_key = False
            else:
                if capturing_key:
                    key_buffer.append(ch)
        else:
            if ch == '"':
                in_string = True
                if depth == 1:
                    capturing_key = True
                    key_buffer = []
            elif ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    break
            elif ch == ":" and capturing_key and depth == 1:
                capturing_key = False
        i += 1

    return True, keys


def extract_property_block(block_text: str, prop_name: str) -> str | None:
    token = f'"{prop_name}"'
    start = block_text.find(token)
    if start == -1:
        return None
    brace_start = block_text.find("{", start)
    if brace_start == -1:
        return None

    depth = 0
    in_string = False
    escape = False
    i = brace_start
    while i < len(block_text):
        ch = block_text[i]
        if in_string:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_string = False
        else:
            if ch == '"':
                in_string = True
            elif ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    return block_text[brace_start : i + 1]
        i += 1
    return None


def extract_enum(block_text: str) -> list[str] | None:
    m = re.search(r'"enum"\s*:\s*\[(.*?)\]', block_text, re.S)
    if not m:
        return None
    return re.findall(r'"([^"]+)"', m.group(1))


def extract_default(block_text: str) -> str | None:
    m = re.search(r'"default"\s*:\s*("[^"]*"|true|false|-?\d+(?:\.\d+)?)', block_text)
    if not m:
        return None
    val = m.group(1)
    return val.strip('"')


def extract_type(block_text: str) -> str | None:
    m = re.search(r'"type"\s*:\s*"([^"]+)"', block_text)
    if not m:
        return None
    return m.group(1)


def default_matches_type(default: str, type_name: str) -> bool:
    if type_name == "string":
        return True
    if type_name == "boolean":
        return default in {"true", "false"}
    if type_name == "integer":
        return re.fullmatch(r"-?\d+", default) is not None
    if type_name == "number":
        return re.fullmatch(r"-?\d+(\.\d+)?", default) is not None
    if type_name in {"array", "object", "null"}:
        return True
    return True


def has_sort_object(block_text: str) -> bool:
    if '"sort"' not in block_text:
        return False
    return '"oneOf"' in block_text and '"type": "object"' in block_text


def main() -> int:
    text = TOOLS_PATH.read_text()
    dispatch_names = parse_dispatch_names(text)
    tool_blocks = parse_tool_blocks(text)

    tool_names = []
    issues = []

    for block in tool_blocks:
        block_text = "\n".join(block["lines"])
        name = extract_name(block)
        if not name:
            continue
        tool_names.append(name)

        if '"inputSchema"' not in block_text:
            issues.append(f"{name}: missing inputSchema")
            continue
        if '"type": "object"' not in block_text:
            issues.append(f"{name}: inputSchema type is not object")

        required = extract_required(block_text)
        has_properties, properties = extract_properties(block_text)
        if not has_properties:
            issues.append(f"{name}: missing properties in inputSchema")
        elif required:
            missing_props = [r for r in required if r not in properties]
            if missing_props:
                issues.append(f"{name}: required not in properties: {missing_props}")

        for prop in properties:
            prop_block = extract_property_block(block_text, prop)
            if not prop_block:
                continue
            enum_values = extract_enum(prop_block)
            default = extract_default(prop_block)
            type_name = extract_type(prop_block)

            if enum_values and default is not None:
                if default not in enum_values:
                    issues.append(f"{name}: default for {prop} not in enum")

            if default is not None and type_name:
                if not default_matches_type(default, type_name):
                    issues.append(f"{name}: default type mismatch for {prop}")

            if prop in LIMIT_FIELDS and type_name not in {None, "integer"}:
                issues.append(f"{name}: {prop} should be integer")

        if name in OPTIONAL_QUERY_TOOLS and "query" in required:
            issues.append(f"{name}: query should be optional")

        if name in SORT_OBJECT_TOOLS and not has_sort_object(block_text):
            issues.append(f"{name}: sort should allow object form")

    tool_name_set = set(tool_names)
    missing_in_list = sorted(dispatch_names - tool_name_set)
    missing_in_dispatch = sorted(tool_name_set - dispatch_names)

    if missing_in_list:
        issues.append(f"Tools in dispatch but missing in tools/list: {missing_in_list}")
    if missing_in_dispatch:
        issues.append(f"Tools in tools/list but missing in dispatch: {missing_in_dispatch}")

    if issues:
        print("Schema checks found issues:")
        for issue in issues:
            print(f"- {issue}")
        return 1

    print("Schema checks passed: no issues found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
