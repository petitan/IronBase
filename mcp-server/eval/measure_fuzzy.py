import json, urllib.request, math, unicodedata, re, time

def post(p, tries=3):
    last = None
    for _ in range(tries):
        try:
            req = urllib.request.Request('http://localhost:8080/mcp', data=json.dumps(p).encode(),
                                         headers={'Content-Type': 'application/json'})
            return json.loads(urllib.request.urlopen(req, timeout=300).read().decode())
        except Exception as e:
            last = e; time.sleep(1.0)
    raise last

post({'jsonrpc': '2.0', 'id': 0, 'method': 'initialize',
      'params': {'protocolVersion': '2024-11-05', 'capabilities': {},
                 'clientInfo': {'name': 'x', 'version': '1'}}})

def tool(name, args, tries=3):
    for _ in range(tries):
        r = post({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                  'params': {'name': name, 'arguments': args}})
        res = r['result']
        if not res.get('isError'):
            return json.loads(res['content'][0]['text'])
        time.sleep(1.0)
    return None

def script(code):
    out = tool('script_exec', {'code': code})
    return out.get('result') if out else None

N = 18987
FILLER = {'egy', 'az', 'a', 'mennyibe', 'kerul', 'kerek', 'adj', 'nehany', 'konkret',
          'kerem', 'generalj', 'keszits', 'sorolj', 'fel', 'mi', 'mit', 'milyen', 'van',
          'es', 'ara', 'arak', 'keress', 'legujabb', 'legutolso', 'utolso'}

def norm(s):
    s = unicodedata.normalize('NFKD', s.lower())
    return ''.join(c for c in s if not unicodedata.combining(c))

def qtokens(q):
    toks = [t for t in re.split(r'[^a-z0-9]+', norm(q)) if len(t) >= 3 and t not in FILLER]
    return list(dict.fromkeys(toks))

def stem(t):
    return t[:6]

# --- Build full doc_id -> normalized text map in ONE paginated pass (no per-doc scans) ---
print('[build] fetching all rdocs chunks (paginated)...', flush=True)
DOCMAP = {}
PAGE = 4000
skip = 0
t0 = time.time()
while True:
    r = tool('find', {'collection': 'rdocs', 'filter': {},
                      'projection': {'content_text': 1, 'doc_id': 1, '_id': 0},
                      'limit': PAGE, 'skip': skip})
    docs = r.get('documents', []) if r else []
    if not docs:
        break
    for c in docs:
        did = c.get('doc_id')
        if did:
            DOCMAP[did] = DOCMAP.get(did, '') + ' ' + norm(str(c.get('content_text', '')))
    skip += len(docs)
    if len(docs) < PAGE:
        break
print('[build] %d docs, %d chunks scanned in %.1fs' % (len(DOCMAP), skip, time.time() - t0), flush=True)

idf_cache = {}
def idf(tok):
    if tok not in idf_cache:
        r = tool('fulltext_search', {'collection': 'rdocs', 'query': tok,
                                     'field': 'content_text', 'limit': 1, 'mode': 'or'})
        df = r.get('count', 0) if r else 0
        idf_cache[tok] = math.log((N + 1) / (df + 1)) + 1.0
    return idf_cache[tok]

word_cache = {}
def coverage(doc_id, qtoks):
    if not qtoks:
        return 1.0
    txt = DOCMAP.get(doc_id, '')
    if doc_id not in word_cache:
        word_cache[doc_id] = set(txt.split())
    words = word_cache[doc_id]
    present = total = 0.0
    for t in qtoks:
        w = idf(t); total += w
        if t in txt or any(dw.startswith(stem(t)) for dw in words):
            present += w
    return present / total if total else 1.0

def or_pool(query):
    # Rhai-side projection: return ONLY doc_id+score (not the full grouped payload
    # with chunks/embeddings, which blows the 10MB script result cap).
    qs = json.dumps(query)
    code = ('let r = db_hybrid_search("rdocs", %s, #{limit: 20, group_by_document: true, '
            'mode: "or", rerank: false}); let o=[]; for g in r { o.push(#{d: g.doc_id, s: g.best_score}); } o' % qs)
    arr = script(code)
    return [{'doc_id': g['d'], 'rrf': g.get('s', 0.0)} for g in arr if g.get('d')] if arr else []

def and_ranking(query):
    qs = json.dumps(query)
    code = ('let r = db_hybrid_search("rdocs", %s, #{limit: 20, group_by_document: true}); '
            'let o=[]; for g in r { o.push(g.doc_id); } o' % qs)
    arr = script(code)
    return [x for x in arr if x] if arr else None

def ndcg(ret, rel, k):
    if not rel: return 0.0
    dcg = sum(1 / math.log2(i + 2) for i, x in enumerate(ret[:k]) if x in rel)
    idcg = sum(1 / math.log2(i + 2) for i in range(min(len(rel), k)))
    return dcg / idcg if idcg else 0.0
def recall(ret, rel, k):
    return sum(1 for r in rel if r in set(ret[:k])) / len(rel) if rel else 0.0
def prec(ret, rel, k):
    top = ret[:k]; return sum(1 for x in top if x in rel) / len(top) if top else 0.0

d = json.load(open('labeled_rdocs_curated_v2.json'))
ans = [x for x in d if x.get('relevant_doc_ids')]
A_VALUES = [1.0, 0.7, 0.5, 0.3, 0.0]
MODES = ['AND(server)'] + ['fuzzy_a%.1f' % a for a in A_VALUES]
agg = {(l, m): {'p': 0, 'r5': 0, 'r10': 0, 'nd': 0, 'n': 0}
       for l in ['specific', 'browse', 'ALL'] for m in MODES}
skipped = []
for x in ans:
    rel = set(x['relevant_doc_ids']); nrel = len(rel)
    pool = or_pool(x['query']); and_ids = and_ranking(x['query'])
    if not pool or and_ids is None:
        skipped.append(x['query']); continue
    qtoks = qtokens(x['query'])
    for c in pool:
        c['mu'] = coverage(c['doc_id'], qtoks)
    rankings = {'AND(server)': and_ids}
    for a in A_VALUES:
        rankings['fuzzy_a%.1f' % a] = [c['doc_id'] for c in sorted(pool, key=lambda c: -(c['rrf'] * (a + (1 - a) * c['mu'])))]
    for m, ret in rankings.items():
        p, r5, r10, nd = prec(ret, rel, 5), recall(ret, rel, 5), recall(ret, rel, 10), ndcg(ret, rel, 5)
        for label, sel in [('specific', nrel <= 5), ('browse', nrel > 5), ('ALL', True)]:
            if sel:
                aa = agg[(label, m)]
                aa['p'] += p; aa['r5'] += r5; aa['r10'] += r10; aa['nd'] += nd; aa['n'] += 1

print('SOLIDIFIED v3: AND=real server; μ from FULL doc text (in-mem map); a=1.0≡OR')
print('answerable=%d evaluated=%d skipped=%d %s' % (len(ans), len(ans) - len(skipped), len(skipped), skipped or ''))
print('%-10s %-12s %4s %8s %9s %10s %8s' % ('bucket', 'mode', 'n', 'P@5', 'Recall@5', 'Recall@10', 'nDCG@5'))
for label in ['specific', 'browse', 'ALL']:
    for m in MODES:
        a = agg[(label, m)]; n = max(1, a['n'])
        print('%-10s %-12s %4d %8.3f %9.3f %10.3f %8.3f' % (label, m, a['n'], a['p']/n, a['r5']/n, a['r10']/n, a['nd']/n))
