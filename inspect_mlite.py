import argparse
import struct
import json
from pathlib import Path

HEADER_SIZE = 256

FIELDS_STRUCT = struct.Struct('<8sIIIQQQQ')

def read_header(f):
    header_bytes = f.read(HEADER_SIZE)
    if len(header_bytes) != HEADER_SIZE:
        raise RuntimeError('file too small for header')
    magic, version, page_size, collection_count, free_list_head, index_offset, metadata_offset, metadata_size = FIELDS_STRUCT.unpack_from(header_bytes)
    return {
        'magic': magic,
        'version': version,
        'page_size': page_size,
        'collection_count': collection_count,
        'free_list_head': free_list_head,
        'index_section_offset': index_offset,
        'metadata_offset': metadata_offset,
        'metadata_size': metadata_size,
    }


def dump_entries(f, start, metadata_offset, max_entries=5):
    entries = []
    offset = start
    idx = 0
    while True:
        if metadata_offset and offset >= metadata_offset:
            break
        f.seek(offset)
        length_bytes = f.read(4)
        if len(length_bytes) < 4:
            break
        length = struct.unpack('<I', length_bytes)[0]
        if length == 0 or length > 100_000_000:
            break
        data = f.read(length)
        if len(data) < length:
            break
        snippet = data[:80]
        entries.append({
            'index': idx,
            'offset': offset,
            'length': length,
            'preview': snippet.decode('utf-8', errors='replace')
        })
        idx += 1
        offset = f.tell()
        if idx >= max_entries:
            break
    return entries


def read_metadata(f, metadata_offset, metadata_size):
    if metadata_offset == 0:
        return None
    f.seek(metadata_offset)
    content = f.read(metadata_size)
    collections = []
    view = memoryview(content)
    pos = 0
    if len(view) < 4:
        return None
    coll_count = struct.unpack_from('<I', view, pos)[0]
    pos += 4
    for _ in range(coll_count):
        if pos + 4 > len(view):
            break
        length = struct.unpack_from('<I', view, pos)[0]
        pos += 4
        data = bytes(view[pos:pos+length])
        pos += length
        try:
            coll = json.loads(data)
        except json.JSONDecodeError:
            coll = {'raw': data.decode('utf-8', errors='replace').strip()}
        collections.append(coll)
    return collections


def main():
    parser = argparse.ArgumentParser(description='Inspect IronBase .mlite file structure')
    parser.add_argument('path', type=Path)
    parser.add_argument('--start', type=int, default=HEADER_SIZE, help='Data scan start offset')
    args = parser.parse_args()

    with args.path.open('rb') as f:
        header = read_header(f)
        print('Header:')
        for k, v in header.items():
            print(f'  {k}: {v}')

        entries = dump_entries(f, args.start, header['metadata_offset'])
        print(f'\nFirst {len(entries)} data entries:')
        for entry in entries:
            preview = entry['preview'].replace('\n', ' ')
            print(f"  idx={entry['index']} offset={entry['offset']} len={entry['length']} preview={preview[:60]}")

        metadata = read_metadata(f, header['metadata_offset'], header['metadata_size'])
        if metadata is None:
            print('\nNo metadata found or metadata offset zero.')
        else:
            print(f"\nMetadata collections ({len(metadata)}):")
            for coll in metadata:
                if isinstance(coll, dict):
                    print(f"  name={coll.get('name')} docs={coll.get('document_count')} last_id={coll.get('last_id')}")
                    print("    raw:", json.dumps(coll, indent=2)[:200])
                    catalog = coll.get('document_catalog')
                    if isinstance(catalog, dict):
                        iterator = catalog.items()
                    elif isinstance(catalog, list):
                        def iter_entries():
                            for entry in catalog:
                                if isinstance(entry, dict):
                                    yield entry.get('key'), entry.get('value')
                                elif isinstance(entry, list) and len(entry) == 2:
                                    yield entry[0], entry[1]
                        iterator = iter_entries()
                    else:
                        if catalog is not None:
                            print(f"    catalog (unsupported format): {type(catalog).__name__} -> {catalog!r}")
                        iterator = iter(())
                    for idx, pair in enumerate(iterator):
                        if not pair or pair[0] is None:
                            continue
                        doc_id, offset = pair
                        print(f"    catalog[{doc_id}] -> offset {offset}")
                        if idx >= 4:
                            print("    ...")
                            break
                else:
                    print(f"  raw entry: {coll!r}")

if __name__ == '__main__':
    main()
