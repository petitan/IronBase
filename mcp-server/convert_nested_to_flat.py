#!/usr/bin/env python3
"""
Convert nested DOCJL format to flat format for MCP server import.

Input: Nested format with children hierarchy
Output: Flat list of blocks with auto-generated labels
"""

import json
import sys
from datetime import datetime

def flatten_blocks(blocks, parent_prefix="", counters=None, heading_level=1):
    """
    Recursively flatten nested blocks into a flat list.

    Args:
        blocks: List of blocks (may have children)
        parent_prefix: Prefix for child blocks (e.g., "sec:1")
        counters: Dict tracking label counters
        heading_level: Current heading nesting level (1-6)

    Returns:
        List of flattened blocks with labels
    """
    if counters is None:
        counters = {
            'sec': 0,     # sections (headings)
            'para': 0,    # paragraphs
            'fig': 0,     # figures
            'tbl': 0,     # tables
            'eq': 0,      # equations
            'lst': 0,     # lists
        }

    flat_blocks = []

    for block in blocks:
        block_type = block.get('type')

        # Normalize list types (list_unordered, list_ordered -> list)
        if block_type in ('list_unordered', 'list_ordered'):
            block_type = 'list'

        # Determine label prefix based on type
        if block_type == 'heading':
            label_prefix = 'sec'
        elif block_type == 'paragraph':
            label_prefix = 'para'
        elif block_type == 'figure':
            label_prefix = 'fig'
        elif block_type == 'table':
            label_prefix = 'tbl'
        elif block_type == 'equation':
            label_prefix = 'eq'
        elif block_type == 'list':
            label_prefix = 'lst'
        else:
            label_prefix = 'para'  # default to paragraph

        # Increment counter and generate label
        counters[label_prefix] += 1
        label = f"{label_prefix}:{counters[label_prefix]}"

        # Normalize content field
        content = block.get('content', [])

        # If content is string, wrap in array format
        if isinstance(content, str):
            content = [{"type": "text", "content": content}]
        elif not isinstance(content, list):
            content = []

        # Create flat block
        flat_block = {
            'type': block_type,
            'label': label,
            'content': content
        }

        # Add block-type specific required fields
        if block_type == 'heading':
            # Heading requires 'level' field (1-6)
            flat_block['level'] = min(heading_level, 6)  # Cap at 6
        elif block_type == 'list':
            # List requires 'ordered' field and 'items' instead of 'content'
            # Determine if ordered based on original type
            original_type = block.get('type')
            flat_block['ordered'] = (original_type == 'list_ordered')
            # Convert content to items format
            if isinstance(content, list):
                flat_block['items'] = [{'content': item} if not isinstance(item, dict) else item for item in content]
            else:
                flat_block['items'] = []
            # Remove content field for lists
            del flat_block['content']
        elif block_type == 'table':
            # Table requires 'headers' and 'rows'
            # Copy from block if present, otherwise create empty structures
            flat_block['headers'] = block.get('headers', [])
            flat_block['rows'] = block.get('rows', [])

        # Add optional fields if present
        if 'metadata' in block:
            flat_block['metadata'] = block['metadata']
        if 'compliance_note' in block:
            flat_block['compliance_note'] = block['compliance_note']

        flat_blocks.append(flat_block)

        # Process children recursively (increment heading level for nested headings)
        if 'children' in block and block['children']:
            next_level = heading_level + 1 if block_type == 'heading' else heading_level
            child_blocks = flatten_blocks(
                block['children'],
                parent_prefix=label,
                counters=counters,
                heading_level=next_level
            )
            flat_blocks.extend(child_blocks)

    return flat_blocks


def convert_nested_to_flat(input_file, output_file=None):
    """
    Convert nested DOCJL file to flat format.

    Args:
        input_file: Path to nested DOCJL JSON file
        output_file: Path to output flat DOCJL JSON (optional)

    Returns:
        Flat document dict
    """
    print(f"Loading nested DOCJL from: {input_file}")

    with open(input_file, 'r', encoding='utf-8') as f:
        nested_data = json.load(f)

    # Extract metadata
    version = nested_data.get('version', '1.0.0')
    format_type = nested_data.get('format', 'nested')

    print(f"  Version: {version}")
    print(f"  Format: {format_type}")

    # Flatten blocks
    print("Flattening blocks...")
    flat_blocks = flatten_blocks(nested_data.get('docjll', []))

    print(f"  Total blocks: {len(flat_blocks)}")

    # Count by type
    type_counts = {}
    for block in flat_blocks:
        block_type = block['type']
        type_counts[block_type] = type_counts.get(block_type, 0) + 1

    print("  Block types:")
    for block_type, count in sorted(type_counts.items()):
        print(f"    {block_type}: {count}")

    # Create flat document
    flat_document = {
        'id': 'mk_manual_v1',
        'metadata': {
            'title': 'Minőségirányítási Kézikönyv',
            'author': 'PeTitan Kft.',
            'version': version,
            'created_at': datetime.now().isoformat(),
            'modified_at': datetime.now().isoformat(),
            'blocks_count': len(flat_blocks),
            'format': 'flat'
        },
        'docjll': flat_blocks  # Use 'docjll' to match server expectation
    }

    # Write output if specified
    if output_file:
        print(f"Writing flat DOCJL to: {output_file}")
        with open(output_file, 'w', encoding='utf-8') as f:
            json.dump(flat_document, f, indent=2, ensure_ascii=False)
        print("✅ Conversion complete!")

    return flat_document


def main():
    if len(sys.argv) < 2:
        print("Usage: python convert_nested_to_flat.py <input.json> [output.json]")
        print("Example: python convert_nested_to_flat.py nested.json flat.json")
        sys.exit(1)

    input_file = sys.argv[1]
    output_file = sys.argv[2] if len(sys.argv) > 2 else None

    if not output_file:
        # Auto-generate output filename
        output_file = input_file.replace('.json', '_flat.json')

    convert_nested_to_flat(input_file, output_file)


if __name__ == '__main__':
    main()
