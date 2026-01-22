#!/usr/bin/env python3
"""
RAG Query Script - Hibrid keresés + Claude API

Használat:
    python rag_query.py "Mi a kalibrálás menete?"
    python rag_query.py "Milyen gázokat kell használni?" --collection gazelemzok_kalibralasi
    python rag_query.py "query" --vector-weight 0.3 --fulltext-weight 0.7

Környezeti változók:
    ANTHROPIC_API_KEY - Claude API kulcs
    MCP_SERVER_URL - MCP szerver (default: http://127.0.0.1:8080/mcp)
"""

import argparse
import json
import os
import sys
import urllib.request
from typing import Optional

# ============ KONFIGURÁCIÓ ============

MCP_URL = os.environ.get("MCP_SERVER_URL", "http://127.0.0.1:8080/mcp")
ANTHROPIC_API_KEY = os.environ.get("ANTHROPIC_API_KEY", "")
CLAUDE_MODEL = "claude-sonnet-4-20250514"
MAX_CONTEXT_CHARS = 12000  # ~3000 token


# ============ MCP KLIENS ============

class MCPClient:
    """IronBase MCP szerver kliens."""

    def __init__(self, url: str = MCP_URL):
        self.url = url
        self.request_id = 0
        self._initialize()

    def _initialize(self):
        """MCP session inicializálása."""
        self._call_raw("initialize", {
            "protocolVersion": "2024-11-05",
            "clientInfo": {"name": "rag_query", "version": "1.0"},
            "capabilities": {}
        })

    def _call_raw(self, method: str, params: dict) -> dict:
        """Raw JSON-RPC hívás."""
        self.request_id += 1
        payload = {
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
            "params": params
        }
        req = urllib.request.Request(
            self.url,
            data=json.dumps(payload).encode('utf-8'),
            headers={'Content-Type': 'application/json'}
        )
        with urllib.request.urlopen(req, timeout=120) as resp:
            return json.load(resp)

    def call_tool(self, tool_name: str, arguments: dict) -> dict:
        """MCP tool hívás."""
        result = self._call_raw("tools/call", {
            "name": tool_name,
            "arguments": arguments
        })
        if "error" in result:
            raise Exception(f"MCP error: {result['error']}")
        content = result.get('result', {}).get('content', [])
        if content and 'text' in content[0]:
            return json.loads(content[0]['text'])
        return {}

    def embed_text(self, text: str) -> list:
        """Szöveg embedding generálás."""
        try:
            result = self.call_tool("embed_batch", {"texts": [text]})
            vectors = result.get('vectors', [[]])
            if vectors and vectors[0]:
                return vectors[0]
        except Exception as e:
            print(f"  [Embedding] MCP embed_batch failed: {e}")

        # Fallback: try to load from cache file
        import hashlib
        cache_file = f"/tmp/embed_cache_{hashlib.md5(text.encode()).hexdigest()[:8]}.json"
        try:
            import os
            if os.path.exists(cache_file):
                with open(cache_file) as f:
                    return json.load(f)
        except Exception:
            pass

        return []

    def vector_search(self, collection: str, field: str, vector: list, limit: int = 5) -> list:
        """Vector similarity keresés."""
        result = self.call_tool("vector_search", {
            "collection": collection,
            "field": field,
            "vector": vector,
            "limit": limit,
            "projection": {"embedding": 0}
        })
        return result.get('results', [])

    def fulltext_search(self, collection: str, field: str, query: str, limit: int = 5) -> list:
        """Fulltext keresés (ha van FTS index)."""
        try:
            result = self.call_tool("fulltext_search", {
                "collection": collection,
                "field": field,
                "query": query,
                "limit": limit,
                "projection": {"embedding": 0}
            })
            # Fulltext response: results[].document + results[].score
            # Flatten to match vector search format
            flat_results = []
            for r in result.get('results', []):
                doc = r.get('document', {})
                doc['_score'] = r.get('score', 0)  # MCP returns 'score', not '_score'
                doc['_matched_tokens'] = r.get('matched_tokens', [])  # MCP returns 'matched_tokens'
                flat_results.append(doc)
            return flat_results
        except Exception:
            # Nincs FTS index, fallback regex keresésre
            return []

    def find_by_regex(self, collection: str, field: str, query: str, limit: int = 5) -> list:
        """Regex alapú keresés fallback."""
        import re

        # Magyar stop words (gyakori, nem informatív szavak)
        STOP_WORDS = {
            'milyen', 'milyenek', 'vannak', 'voltak', 'lenne', 'lennének',
            'hogy', 'mint', 'vagy', 'amely', 'amelyek', 'amit', 'amiket',
            'ahol', 'amikor', 'mivel', 'mert', 'tehát', 'vagyis', 'illetve',
            'valamint', 'pedig', 'csak', 'még', 'már', 'minden', 'összes',
            'egyik', 'másik', 'szerint', 'között', 'után', 'előtt', 'alatt',
            'felett', 'mellett', 'által', 'felé', 'ellen', 'miatt', 'helyett'
        }

        # Keresési szavak kinyerése (min 4 karakter, nem stop word)
        words = []
        for w in query.split():
            # Escape regex metacharacters és tisztítás
            clean = re.sub(r'[?.*+^${}()|[\]\\]', '', w.lower())
            if len(clean) >= 4 and clean not in STOP_WORDS:
                words.append(clean)

        if not words:
            return []

        # OR pattern a szavakra
        pattern = "|".join(words)
        result = self.call_tool("find", {
            "collection": collection,
            "filter": {field: {"$regex": pattern, "$options": "i"}},
            "limit": limit,
            "projection": {"embedding": 0}
        })
        return result.get('documents', [])

    def hybrid_search(
        self,
        collection: str,
        vector_field: str,
        text_field: str,
        vector: list,
        query: str,
        limit: int = 10,
        vector_weight: float = 0.5,
        fulltext_weight: float = 0.5,
        projection: Optional[dict] = None
    ) -> list:
        """
        Hybrid search using MCP hybrid_search tool (RRF-based fusion).

        Args:
            collection: Collection name
            vector_field: Field with vector index
            text_field: Field with fulltext index
            vector: Query embedding vector
            query: Text query for fulltext search
            limit: Maximum results
            vector_weight: Weight for vector search (0-1)
            fulltext_weight: Weight for fulltext search (0-1)
            projection: Optional projection

        Returns:
            List of results with _rrf_score, _vector_rank, _text_rank
        """
        args = {
            "collection": collection,
            "vector_field": vector_field,
            "text_field": text_field,
            "vector": vector,
            "query": query,
            "limit": limit,
            "vector_weight": vector_weight,
            "fulltext_weight": fulltext_weight
        }
        if projection:
            args["projection"] = projection

        result = self.call_tool("hybrid_search", args)
        return result.get('results', [])


# ============ QUERY PREPROCESSING ============

# Magyar kérdőszavak és stop words
HUNGARIAN_QUESTION_WORDS = {
    'milyen', 'milyenek', 'hogyan', 'miért', 'mikor', 'hol', 'honnan', 'hová',
    'mennyi', 'hány', 'melyik', 'melyeket', 'kik', 'kiket', 'mik', 'miket'
}

HUNGARIAN_STOP_WORDS = {
    'vannak', 'voltak', 'lenne', 'lennének', 'lesz', 'lesznek',
    'hogy', 'mint', 'vagy', 'amely', 'amelyek', 'amit', 'amiket',
    'ahol', 'amikor', 'mivel', 'mert', 'tehát', 'vagyis', 'illetve',
    'valamint', 'pedig', 'csak', 'még', 'már', 'minden', 'összes',
    'egyik', 'másik', 'szerint', 'között', 'után', 'előtt', 'alatt',
    'felett', 'mellett', 'által', 'felé', 'ellen', 'miatt', 'helyett',
    'ezek', 'azok', 'ilyen', 'olyan', 'igen', 'nem', 'vagy', 'és'
}

def preprocess_query(query: str) -> str:
    """
    Magyar természetes nyelvi kérdés előfeldolgozása.
    Eltávolítja a kérdőszavakat, stop words-öket és ragokat.
    """
    import re

    # Magyar ragok eltávolítása (egyszerűsített)
    SUFFIXES = [
        'nek', 'nak', 'ből', 'ból', 'től', 'tól', 'hez', 'hoz', 'höz',
        'ban', 'ben', 'ba', 'be', 'ra', 're', 'ról', 'ről', 'val', 'vel',
        'ért', 'ként', 'kor', 'ig', 'nál', 'nél',
        'ok', 'ek', 'ök', 'ak',  # többes szám
        'i', 'ai', 'ei', 'jai', 'jei',  # birtokos többes
        'ja', 'je', 'a', 'e',  # birtokos egyes
    ]

    def strip_suffix(word: str) -> str:
        """Eltávolítja a leggyakoribb magyar ragokat."""
        for suffix in sorted(SUFFIXES, key=len, reverse=True):
            if len(word) > len(suffix) + 2 and word.endswith(suffix):
                return word[:-len(suffix)]
        return word

    # Tisztítás
    query = re.sub(r'[?!.,;:]', '', query.lower())

    # Szavak szűrése és szótövesítés
    words = []
    for word in query.split():
        if word in HUNGARIAN_QUESTION_WORDS:
            continue
        if word in HUNGARIAN_STOP_WORDS:
            continue
        if len(word) < 3:
            continue

        # Rag eltávolítás
        stem = strip_suffix(word)
        if len(stem) >= 3:
            words.append(stem)
        else:
            words.append(word)

    return ' '.join(words)


# ============ HIBRID KERESÉS ============

def rerank_results(results: list, query: str, text_field: str = "content") -> list:
    """
    Lightweight reranking: keyword overlap, phrase match, heading boost.

    Args:
        results: Hybrid search eredmények
        query: Eredeti query
        text_field: Szöveges mező

    Returns:
        Újrarangsorolt lista
    """
    import re

    # Query szavak (lowercase, min 3 char)
    query_words = set(w.lower() for w in re.findall(r'\w+', query) if len(w) >= 3)
    # Query stems (first 5+ chars for Hungarian matching)
    query_stems = set(w[:min(len(w), 6)] for w in query_words if len(w) >= 4)
    query_lower = query.lower()

    for item in results:
        doc = item['doc']
        content = doc.get(text_field, '').lower()
        heading = doc.get('heading', '').lower()

        boost_mult = 1.0  # Multiplicative boost for RRF compatibility

        # 1. Heading boost: query stems a címben (1.5x per stem)
        for stem in query_stems:
            if stem in heading:
                boost_mult *= 1.5
                break  # Only one boost per heading

        # 2. Exact phrase boost: teljes query megtalálható (1.3x)
        if len(query_lower) > 10 and query_lower in content:
            boost_mult *= 1.3

        # 3. Keyword density: query szavak / összes szó
        content_words = re.findall(r'\w+', content)
        if content_words:
            matches = sum(1 for w in content_words if w in query_words)
            density = matches / len(content_words)
            boost_mult *= (1.0 + density)  # max ~1.1x boost

        # 4. Content length penalty: túl rövid tartalom (0.8x)
        if len(content) < 100:
            boost_mult *= 0.8

        # 5. Section path boost: releváns szekció (1.1x)
        section_path = ' '.join(doc.get('section_path', [])).lower()
        for word in query_words:
            if word in section_path:
                boost_mult *= 1.1
                break

        item['rerank_boost'] = boost_mult
        item['final_score'] = item['score'] * boost_mult

    # Újrarendezés final_score alapján
    results.sort(key=lambda x: x['final_score'], reverse=True)
    return results


def deduplicate_results(results: list, text_field: str = "content", similarity_threshold: int = 100) -> list:
    """
    Deduplikálja a találatokat tartalom alapján.

    Args:
        results: Rendezett találatok listája
        text_field: Szöveges mező neve
        similarity_threshold: Hány karakter egyezés számít duplikátnak

    Returns:
        Deduplikált lista
    """
    seen_prefixes = set()
    seen_headings = set()
    unique_results = []

    for item in results:
        # Content-based dedup
        content = item['doc'].get(text_field, '')[:similarity_threshold]

        # Heading-based dedup (same heading = likely same section)
        heading = item['doc'].get('heading', '').strip()

        # Skip if content OR heading already seen
        if content in seen_prefixes:
            continue
        if heading and heading in seen_headings:
            continue

        seen_prefixes.add(content)
        if heading:
            seen_headings.add(heading)
        unique_results.append(item)

    return unique_results


def hybrid_search(
    mcp: MCPClient,
    collection: str,
    query: str,
    vector_field: str = "embedding",
    text_field: str = "content",
    limit: int = 10,
    vector_weight: float = 0.4,
    fulltext_weight: float = 0.6,
    min_score: float = 0.0,
    deduplicate: bool = True,
    rerank: bool = True
) -> list:
    """
    Hibrid keresés: MCP hybrid_search tool használatával (RRF-based fusion).

    Args:
        mcp: MCP kliens
        collection: Kollekció neve
        query: Keresési szöveg
        vector_field: Embedding mező neve
        text_field: Szöveges mező neve
        limit: Max találatok száma
        vector_weight: Vector search súlya (0-1)
        fulltext_weight: Fulltext search súlya (0-1)
        min_score: Minimum score cutoff (default: 0.0)
        deduplicate: Duplikátumok eltávolítása (default: True)
        rerank: Lightweight reranking (default: True)

    Returns:
        Rangsorolt találatok listája
    """
    # Query előfeldolgozás - kérdőszavak és stop words eltávolítása
    processed_query = preprocess_query(query)
    print(f"  [Query] original: '{query}'")
    print(f"  [Query] processed: '{processed_query}'")

    # 1. Query embedding generálás
    print(f"  [Embedding] generating query vector...")
    query_embedding = mcp.embed_text(query)

    if not query_embedding:
        print("  [ERROR] Failed to generate embedding")
        return []

    # 2. MCP hybrid_search hívás (RRF fusion server-side)
    print(f"  [Hybrid Search] v_weight={vector_weight}, t_weight={fulltext_weight}")
    mcp_results = mcp.hybrid_search(
        collection=collection,
        vector_field=vector_field,
        text_field=text_field,
        vector=query_embedding,
        query=processed_query,
        limit=limit * 3,  # Extra for post-processing
        vector_weight=vector_weight,
        fulltext_weight=fulltext_weight,
        projection={"embedding": 0}  # Exclude large embedding field
    )

    print(f"  [RRF] server-side fusion, {len(mcp_results)} results")

    # 3. Convert MCP results to local format
    results = []
    for doc in mcp_results:
        v_rank = doc.pop('_vector_rank', 999)
        t_rank = doc.pop('_text_rank', 999)
        rrf_score = doc.pop('_rrf_score', 0)
        v_score = doc.pop('_vector_score', None)
        t_score = doc.pop('_text_score', None)

        sources = []
        if v_rank < 100:
            sources.append(f"v_rank:{v_rank}")
        if t_rank < 100:
            sources.append(f"t_rank:{t_rank}")

        results.append({
            'doc': doc,
            'score': rrf_score,
            'sources': sources,
            'v_rank': v_rank,
            't_rank': t_rank,
            'v_score': v_score,
            't_score': t_score
        })

    # 4. Reranking (calculates final_score with multiplicative boost)
    if rerank:
        results = rerank_results(results, query, text_field)
        reranked_any = any(r.get('rerank_boost', 1.0) != 1.0 for r in results)
        if reranked_any:
            print(f"  [Rerank] multiplicative boost applied")

    # 5. Min score filter (uses final_score if available)
    if min_score > 0:
        filtered_count = len(results)
        results = [r for r in results if r.get('final_score', r['score']) >= min_score]
        if filtered_count != len(results):
            print(f"  [Min score filter] {filtered_count} -> {len(results)} (cutoff: {min_score})")

    # 6. Deduplikáció
    if deduplicate:
        before_dedup = len(results)
        results = deduplicate_results(results, text_field)
        if before_dedup != len(results):
            print(f"  [Deduplication] {before_dedup} -> {len(results)} unique")

    return results[:limit]


# ============ CLAUDE API ============

def call_claude(prompt: str, context: str, api_key: str) -> str:
    """Claude API hívás RAG kontextussal."""

    system_prompt = """Te egy szakértő asszisztens vagy, aki a megadott dokumentum kontextus alapján válaszol a kérdésekre.

SZABÁLYOK:
- CSAK a megadott kontextus alapján válaszolj
- Ha a kontextus nem tartalmazza a választ, mondd el őszintén
- Hivatkozz a releváns részekre
- Válaszolj magyarul, szakszerűen de érthetően
- Ha táblázat vagy lista releváns, használd azt a formátumot"""

    user_message = f"""KONTEXTUS:
{context}

KÉRDÉS: {prompt}

Válaszolj a kérdésre a fenti kontextus alapján."""

    payload = {
        "model": CLAUDE_MODEL,
        "max_tokens": 2000,
        "temperature": 0.3,
        "system": system_prompt,
        "messages": [
            {"role": "user", "content": user_message}
        ]
    }

    req = urllib.request.Request(
        "https://api.anthropic.com/v1/messages",
        data=json.dumps(payload).encode('utf-8'),
        headers={
            'Content-Type': 'application/json',
            'x-api-key': api_key,
            'anthropic-version': '2023-06-01'
        }
    )

    with urllib.request.urlopen(req, timeout=60) as resp:
        result = json.load(resp)

    return result['content'][0]['text']


# ============ RAG PIPELINE ============

def rag_query(
    query: str,
    collection: str,
    vector_weight: float = 0.4,
    fulltext_weight: float = 0.6,
    top_k: int = 10,
    min_score: float = 0.0,
    deduplicate: bool = True,
    rerank: bool = True,
    api_key: Optional[str] = None
) -> dict:
    """
    Teljes RAG pipeline: keresés + LLM válasz.

    Args:
        query: Felhasználói kérdés
        collection: Kollekció neve
        vector_weight: Vector keresés súlya
        fulltext_weight: Fulltext keresés súlya
        top_k: Hány kontextus chunk-ot használjon
        min_score: Minimum score cutoff
        deduplicate: Duplikátumok eltávolítása
        rerank: Lightweight reranking alkalmazása
        api_key: Anthropic API kulcs (vagy env-ből)

    Returns:
        {
            'answer': str,
            'sources': list,
            'context_used': int
        }
    """
    api_key = api_key or ANTHROPIC_API_KEY

    if not api_key:
        raise ValueError("ANTHROPIC_API_KEY környezeti változó szükséges!")

    print(f"\n{'='*60}")
    print(f"RAG Query: {query}")
    print(f"Collection: {collection}")
    print(f"{'='*60}")

    # 1. MCP kapcsolat
    print("\n[1] Connecting to MCP server...")
    mcp = MCPClient(MCP_URL)
    print(f"    Connected: {MCP_URL}")

    # 2. Hibrid keresés
    print("\n[2] Hybrid search...")
    results = hybrid_search(
        mcp, collection, query,
        vector_weight=vector_weight,
        fulltext_weight=fulltext_weight,
        limit=top_k,
        min_score=min_score,
        deduplicate=deduplicate,
        rerank=rerank
    )

    print(f"    Found {len(results)} relevant chunks")

    if not results:
        return {
            'answer': "Nem találtam releváns információt a kérdéshez.",
            'sources': [],
            'context_used': 0
        }

    # 3. Kontextus összeállítása
    print("\n[3] Building context...")
    context_parts = []
    sources = []
    total_chars = 0

    for i, item in enumerate(results, 1):
        doc = item['doc']
        score = item['score']
        src = item['sources']

        content = doc.get('content', '')
        heading = doc.get('heading', '')
        section = doc.get('section_path', [])
        chunk_info = doc.get('chunk', {})
        source_info = doc.get('source', {})

        # Chunk header - fallback to chunk index if no heading
        if heading:
            header = f"[{heading}]"
        elif chunk_info:
            header = f"[Chunk {chunk_info.get('index', i)}/{chunk_info.get('total', '?')}]"
        else:
            header = f"[Chunk {i}]"

        if section:
            header += f" ({' > '.join(section[-2:])})"
        elif source_info.get('filename'):
            header += f" ({source_info['filename']})"

        chunk_text = f"{header}\n{content}\n"

        # Limit ellenőrzés
        if total_chars + len(chunk_text) > MAX_CONTEXT_CHARS:
            print(f"    Chunk {i} skipped (context limit)")
            break

        context_parts.append(chunk_text)

        # Better source label
        if heading:
            source_label = heading
        elif chunk_info:
            source_label = f"Chunk {chunk_info.get('index', i)}/{chunk_info.get('total', '?')}"
        else:
            source_label = f"Chunk {i}"

        sources.append({
            'label': source_label,
            'score': round(score, 3),
            'match_sources': src
        })
        total_chars += len(chunk_text)

        print(f"    [{i}] {source_label} (score: {score:.3f})")

    context = "\n---\n".join(context_parts)
    print(f"    Total context: {total_chars} chars (~{total_chars//4} tokens)")

    # 4. Claude API hívás
    print("\n[4] Calling Claude API...")
    print(f"    Model: {CLAUDE_MODEL}")

    answer = call_claude(query, context, api_key)

    print("\n[5] Done!")

    return {
        'answer': answer,
        'sources': sources,
        'context_used': total_chars
    }


# ============ CLI ============

def main():
    parser = argparse.ArgumentParser(
        description="RAG Query - Hibrid keresés + Claude API",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Példák:
  python rag_query.py "Mi a kalibrálás menete?"
  python rag_query.py "Milyen gázokat használnak?" -c gazelemzok_kalibralasi
  python rag_query.py "kérdés" --vector-weight 0.8 --fulltext-weight 0.2
        """
    )

    parser.add_argument("query", help="Keresési kérdés")
    parser.add_argument("-c", "--collection", default="gazelemzok_kalibralasi",
                        help="Kollekció neve (default: gazelemzok_kalibralasi)")
    parser.add_argument("-k", "--top-k", type=int, default=10,
                        help="Hány kontextus chunk (default: 10)")
    parser.add_argument("--vector-weight", type=float, default=0.4,
                        help="Vector search súlya (default: 0.4)")
    parser.add_argument("--fulltext-weight", type=float, default=0.6,
                        help="Fulltext search súlya (default: 0.6)")
    parser.add_argument("--min-score", type=float, default=0.0,
                        help="Minimum score cutoff (default: 0.0, disabled for RRF)")
    parser.add_argument("--no-dedup", action="store_true",
                        help="Deduplikáció kikapcsolása")
    parser.add_argument("--no-rerank", action="store_true",
                        help="Reranking kikapcsolása")
    parser.add_argument("--no-llm", action="store_true",
                        help="Csak keresés, LLM nélkül")

    args = parser.parse_args()

    try:
        if args.no_llm:
            # Csak keresés
            print(f"\n{'='*60}")
            print(f"Hybrid Search (no LLM)")
            print(f"{'='*60}")

            mcp = MCPClient(MCP_URL)
            results = hybrid_search(
                mcp, args.collection, args.query,
                vector_weight=args.vector_weight,
                fulltext_weight=args.fulltext_weight,
                limit=args.top_k,
                min_score=args.min_score,
                deduplicate=not args.no_dedup,
                rerank=not args.no_rerank
            )

            print(f"\n{'='*60}")
            print(f"Results ({len(results)} found)")
            print(f"{'='*60}\n")

            for i, item in enumerate(results, 1):
                doc = item['doc']
                score = item['score']
                chunk_info = doc.get('chunk', {})
                content = doc.get('content', '')[:200]

                # Better label - use heading if available, otherwise chunk index
                heading = doc.get('heading', '')
                if heading:
                    label = heading
                elif chunk_info:
                    label = f"Chunk {chunk_info.get('index', i)}/{chunk_info.get('total', '?')}"
                else:
                    label = f"Chunk {i}"

                print(f"[{i}] {label} (score: {score:.3f})")
                print(f"    Sources: {', '.join(item['sources'])}")
                print(f"    {content}...\n")

        else:
            # Teljes RAG pipeline
            result = rag_query(
                args.query,
                args.collection,
                vector_weight=args.vector_weight,
                fulltext_weight=args.fulltext_weight,
                top_k=args.top_k,
                min_score=args.min_score,
                deduplicate=not args.no_dedup,
                rerank=not args.no_rerank
            )

            print(f"\n{'='*60}")
            print("VÁLASZ")
            print(f"{'='*60}\n")
            print(result['answer'])

            print(f"\n{'='*60}")
            print("FORRÁSOK")
            print(f"{'='*60}")
            for i, src in enumerate(result['sources'], 1):
                print(f"  [{i}] {src['label']} (score: {src['score']})")

            print(f"\nContext: {result['context_used']} chars")

    except Exception as e:
        print(f"\n❌ Hiba: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
