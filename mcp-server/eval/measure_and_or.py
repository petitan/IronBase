import json, urllib.request, math

def post(p):
    req = urllib.request.Request('http://localhost:8080/mcp', data=json.dumps(p).encode(),
                                 headers={'Content-Type': 'application/json'})
    return json.loads(urllib.request.urlopen(req, timeout=120).read().decode())

post({'jsonrpc': '2.0', 'id': 0, 'method': 'initialize',
      'params': {'protocolVersion': '2024-11-05', 'capabilities': {},
                 'clientInfo': {'name': 'x', 'version': '1'}}})

def run(query, mode):
    qs = json.dumps(query)
    modepart = ', mode: "%s"' % mode if mode else ''
    code = 'db_hybrid_search("rdocs", %s, #{limit: 10, group_by_document: true%s})' % (qs, modepart)
    r = post({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
              'params': {'name': 'script_exec', 'arguments': {'code': code}}})
    res = r['result']
    if res.get('isError'):
        return None
    out = json.loads(res['content'][0]['text'])
    return [g.get('doc_id') for g in out.get('result', []) if isinstance(g, dict) and g.get('doc_id')]

def ndcg(ret, rel, k):
    if not rel:
        return 0.0
    dcg = sum(1 / math.log2(i + 2) for i, x in enumerate(ret[:k]) if x in rel)
    idcg = sum(1 / math.log2(i + 2) for i in range(min(len(rel), k)))
    return dcg / idcg if idcg else 0.0

def recall(ret, rel, k):
    return sum(1 for r in rel if r in set(ret[:k])) / len(rel) if rel else 0.0

def prec(ret, rel, k):
    top = ret[:k]
    return sum(1 for x in top if x in rel) / len(top) if top else 0.0

d = json.load(open('labeled_rdocs_curated_v2.json'))
ans = [x for x in d if x.get('relevant_doc_ids')]
agg = {}
for label in ['specific', 'browse', 'ALL']:
    for mode in ['AND', 'OR']:
        agg[(label, mode)] = {'p': 0, 'r5': 0, 'r10': 0, 'nd': 0, 'n': 0}

for x in ans:
    rel = set(x['relevant_doc_ids'])
    nrel = len(rel)
    for mode, mval in [('AND', None), ('OR', 'or')]:
        ret = run(x['query'], mval)
        if ret is None:
            continue
        p, r5, r10, nd = prec(ret, rel, 5), recall(ret, rel, 5), recall(ret, rel, 10), ndcg(ret, rel, 5)
        for label, sel in [('specific', nrel <= 5), ('browse', nrel > 5), ('ALL', True)]:
            if sel:
                a = agg[(label, mode)]
                a['p'] += p; a['r5'] += r5; a['r10'] += r10; a['nd'] += nd; a['n'] += 1

print('%-10s %-4s %4s %8s %9s %10s %8s' % ('bucket', 'mode', 'n', 'P@5', 'Recall@5', 'Recall@10', 'nDCG@5'))
for label in ['specific', 'browse', 'ALL']:
    for mode in ['AND', 'OR']:
        a = agg[(label, mode)]
        n = max(1, a['n'])
        print('%-10s %-4s %4d %8.3f %9.3f %10.3f %8.3f' %
              (label, mode, a['n'], a['p'] / n, a['r5'] / n, a['r10'] / n, a['nd'] / n))
