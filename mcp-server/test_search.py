import requests
import json

# Test search_content endpoint
response = requests.post('http://127.0.0.1:8080/mcp',
    headers={'Content-Type': 'application/json', 'Authorization': 'Bearer dev_key_12345'},
    json={
        'jsonrpc': '2.0',
        'id': 1,
        'method': 'mcp_docjl_search_content',
        'params': {
            'document_id': 'mk_manual_v1',
            'query': 'gázelemző',
            'case_sensitive': False,
            'max_results': 10
        }
    }
)

result = response.json()
print('Full response:', json.dumps(result, indent=2, ensure_ascii=False))
print()
print('=== Search Results for "gázelemző" ===')
if 'error' in result:
    print('ERROR:', result['error'])
elif 'result' in result:
    print('Document ID:', result['result'].get('document_id'))
    print('Query:', result['result'].get('query'))
    print('Total matches:', result['result'].get('total_matches'))
    print()
    if result['result'].get('matches'):
        print('First match:')
        match = result['result']['matches'][0]
        print('  Block index:', match.get('block_index'))
        print('  Block type:', match.get('block_type'))
        print('  Label:', match.get('label'))
        print('  Text preview (first 100 chars):', match.get('text', '')[:100])
else:
    print('Unexpected response format')
