using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using IronBase.Interop;

namespace IronBase
{
    /// <summary>
    /// Represents a collection of documents.
    /// Provides MongoDB-like CRUD operations.
    /// </summary>
    /// <typeparam name="T">Document type</typeparam>
    public sealed class IronBaseCollection<T> where T : class
    {
        private readonly IronBaseClient _client;
        private readonly string _name;
        private IntPtr _handle;

        internal IronBaseCollection(IronBaseClient client, string name)
        {
            _client = client ?? throw new ArgumentNullException(nameof(client));
            _name = name ?? throw new ArgumentNullException(nameof(name));
            InitializeHandle();
        }

        private unsafe void InitializeHandle()
        {
            fixed (byte* namePtr = NativeHelper.ToUtf8(_name))
            {
                CollectionHandle* handlePtr;
                int result = NativeMethods.ironbase_collection(
                    (DatabaseHandle*)_client.Handle,
                    namePtr,
                    &handlePtr
                );
                NativeHelper.ThrowIfError(result);
                _handle = (IntPtr)handlePtr;
            }
        }

        /// <summary>
        /// Collection name.
        /// </summary>
        public string Name => _name;

        // ============== INSERT ==============

        /// <summary>
        /// Insert one document.
        /// </summary>
        public InsertOneResult InsertOne(T document)
        {
            if (document == null)
                throw new ArgumentNullException(nameof(document));

            var json = JsonSerializer.Serialize(document);

            unsafe
            {
                fixed (byte* docPtr = NativeHelper.ToUtf8(json))
                {
                    byte* idPtr;
                    int result = NativeMethods.ironbase_insert_one(
                        (CollectionHandle*)_handle,
                        docPtr,
                        &idPtr
                    );
                    NativeHelper.ThrowIfError(result);

                    return new InsertOneResult
                    {
                        Acknowledged = true,
                        InsertedId = NativeHelper.PtrToStringUtf8AndFree(idPtr)
                    };
                }
            }
        }

        /// <summary>
        /// Insert many documents.
        /// </summary>
        public InsertManyResult InsertMany(IEnumerable<T> documents)
        {
            if (documents == null)
                throw new ArgumentNullException(nameof(documents));

            var docs = new List<T>(documents);
            var json = JsonSerializer.Serialize(docs);

            unsafe
            {
                fixed (byte* docsPtr = NativeHelper.ToUtf8(json))
                {
                    byte* resultPtr;
                    int result = NativeMethods.ironbase_insert_many(
                        (CollectionHandle*)_handle,
                        docsPtr,
                        &resultPtr
                    );
                    NativeHelper.ThrowIfError(result);

                    var resultJson = NativeHelper.PtrToStringUtf8AndFree(resultPtr) ?? "{}";
                    var resultDoc = JsonSerializer.Deserialize<JsonElement>(resultJson);

                    string[]? insertedIds = null;
                    if (resultDoc.TryGetProperty("inserted_ids", out var idsElement))
                    {
                        insertedIds = idsElement.EnumerateArray()
                            .Select(id => id.ToString())
                            .ToArray();
                    }

                    return new InsertManyResult
                    {
                        Acknowledged = true,
                        InsertedCount = resultDoc.TryGetProperty("inserted_count", out var count) ? count.GetInt32() : 0,
                        InsertedIds = insertedIds
                    };
                }
            }
        }

        // ============== FIND ==============

        /// <summary>
        /// Find all documents matching a filter.
        /// </summary>
        public List<T> Find(FilterDefinition<T>? filter = null)
        {
            var filterJson = filter?.ToJson() ?? "{}";

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                {
                    var resultPtr = NativeMethods.ironbase_find(
                        (CollectionHandle*)_handle,
                        filterPtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        if (!string.IsNullOrEmpty(error))
                            throw new IronBaseQueryException(error);
                        return new List<T>();
                    }

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (string.IsNullOrEmpty(json) || json == "[]")
                        return new List<T>();

                    return JsonSerializer.Deserialize<List<T>>(json) ?? new List<T>();
                }
            }
        }

        /// <summary>
        /// Find all documents matching a JSON filter string.
        /// </summary>
        public List<T> Find(string filterJson)
        {
            return Find(new FilterDefinition<T>(filterJson));
        }

        /// <summary>
        /// Find one document matching a filter.
        /// </summary>
        public T? FindOne(FilterDefinition<T>? filter = null)
        {
            var filterJson = filter?.ToJson() ?? "{}";

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                {
                    var resultPtr = NativeMethods.ironbase_find_one(
                        (CollectionHandle*)_handle,
                        filterPtr
                    );

                    if (resultPtr == null)
                        return default;

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (string.IsNullOrEmpty(json))
                        return default;

                    return JsonSerializer.Deserialize<T>(json);
                }
            }
        }

        /// <summary>
        /// Find documents with options (projection, sort, limit, skip).
        /// </summary>
        public List<T> Find(FilterDefinition<T> filter, FindOptions options)
        {
            var filterJson = filter?.ToJson() ?? "{}";
            var optionsJson = options?.ToJson() ?? "{}";

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                fixed (byte* optionsPtr = NativeHelper.ToUtf8(optionsJson))
                {
                    var resultPtr = NativeMethods.ironbase_find_with_options(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        optionsPtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        if (!string.IsNullOrEmpty(error))
                            throw new IronBaseQueryException(error);
                        return new List<T>();
                    }

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (string.IsNullOrEmpty(json) || json == "[]")
                        return new List<T>();

                    return JsonSerializer.Deserialize<List<T>>(json) ?? new List<T>();
                }
            }
        }

        /// <summary>
        /// Find documents with options and optionally include total count for pagination.
        /// </summary>
        /// <param name="filter">Query filter</param>
        /// <param name="options">Find options (use IncludeTotal = true to get total count)</param>
        /// <returns>FindResult with Documents and optional Total count</returns>
        /// <example>
        /// // Paginated query with total count
        /// var options = new FindOptions { Limit = 10, Skip = 20, IncludeTotal = true };
        /// var result = collection.FindWithTotal(new FilterDefinition&lt;User&gt;("{}"), options);
        /// Console.WriteLine($"Showing {result.Documents.Count} of {result.Total} total");
        /// </example>
        public FindResult<T> FindWithTotal(FilterDefinition<T> filter, FindOptions options)
        {
            var filterJson = filter?.ToJson() ?? "{}";
            var optionsJson = options?.ToJson() ?? "{}";

            var result = new FindResult<T>();

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                fixed (byte* optionsPtr = NativeHelper.ToUtf8(optionsJson))
                {
                    var resultPtr = NativeMethods.ironbase_find_with_options(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        optionsPtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        if (!string.IsNullOrEmpty(error))
                            throw new IronBaseQueryException(error);
                        return result;
                    }

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (!string.IsNullOrEmpty(json) && json != "[]")
                    {
                        result.Documents = JsonSerializer.Deserialize<List<T>>(json) ?? new List<T>();
                    }
                }

                // If IncludeTotal is requested, run a separate count query
                if (options?.IncludeTotal == true)
                {
                    fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                    {
                        ulong count;
                        int countResult = NativeMethods.ironbase_count_documents(
                            (CollectionHandle*)_handle,
                            filterPtr,
                            &count
                        );
                        NativeHelper.ThrowIfError(countResult);
                        result.Total = (long)count;
                    }
                }
            }

            return result;
        }

        // ============== UPDATE ==============

        /// <summary>
        /// Update one document.
        /// </summary>
        public UpdateResult UpdateOne(FilterDefinition<T> filter, UpdateDefinition<T> update)
        {
            if (filter == null) throw new ArgumentNullException(nameof(filter));
            if (update == null) throw new ArgumentNullException(nameof(update));

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filter.ToJson()))
                fixed (byte* updatePtr = NativeHelper.ToUtf8(update.ToJson()))
                {
                    ulong matched, modified;
                    int result = NativeMethods.ironbase_update_one(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        updatePtr,
                        &matched,
                        &modified
                    );
                    NativeHelper.ThrowIfError(result);

                    return new UpdateResult
                    {
                        Acknowledged = true,
                        MatchedCount = (long)matched,
                        ModifiedCount = (long)modified
                    };
                }
            }
        }

        /// <summary>
        /// Update many documents.
        /// </summary>
        public UpdateResult UpdateMany(FilterDefinition<T> filter, UpdateDefinition<T> update)
        {
            if (filter == null) throw new ArgumentNullException(nameof(filter));
            if (update == null) throw new ArgumentNullException(nameof(update));

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filter.ToJson()))
                fixed (byte* updatePtr = NativeHelper.ToUtf8(update.ToJson()))
                {
                    ulong matched, modified;
                    int result = NativeMethods.ironbase_update_many(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        updatePtr,
                        &matched,
                        &modified
                    );
                    NativeHelper.ThrowIfError(result);

                    return new UpdateResult
                    {
                        Acknowledged = true,
                        MatchedCount = (long)matched,
                        ModifiedCount = (long)modified
                    };
                }
            }
        }

        // ============== DELETE ==============

        /// <summary>
        /// Delete one document.
        /// </summary>
        public DeleteResult DeleteOne(FilterDefinition<T> filter)
        {
            if (filter == null) throw new ArgumentNullException(nameof(filter));

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filter.ToJson()))
                {
                    ulong deleted;
                    int result = NativeMethods.ironbase_delete_one(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        &deleted
                    );
                    NativeHelper.ThrowIfError(result);

                    return new DeleteResult
                    {
                        Acknowledged = true,
                        DeletedCount = (long)deleted
                    };
                }
            }
        }

        /// <summary>
        /// Delete many documents.
        /// </summary>
        public DeleteResult DeleteMany(FilterDefinition<T> filter)
        {
            if (filter == null) throw new ArgumentNullException(nameof(filter));

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filter.ToJson()))
                {
                    ulong deleted;
                    int result = NativeMethods.ironbase_delete_many(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        &deleted
                    );
                    NativeHelper.ThrowIfError(result);

                    return new DeleteResult
                    {
                        Acknowledged = true,
                        DeletedCount = (long)deleted
                    };
                }
            }
        }

        // ============== COUNT & DISTINCT ==============

        /// <summary>
        /// Count documents matching a filter.
        /// </summary>
        public long CountDocuments(FilterDefinition<T>? filter = null)
        {
            var filterJson = filter?.ToJson() ?? "{}";

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                {
                    ulong count;
                    int result = NativeMethods.ironbase_count_documents(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        &count
                    );
                    NativeHelper.ThrowIfError(result);
                    return (long)count;
                }
            }
        }

        /// <summary>
        /// Get distinct values for a field.
        /// </summary>
        public List<TValue> Distinct<TValue>(string field, FilterDefinition<T>? filter = null)
        {
            var filterJson = filter?.ToJson() ?? "{}";

            unsafe
            {
                fixed (byte* fieldPtr = NativeHelper.ToUtf8(field))
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                {
                    var resultPtr = NativeMethods.ironbase_distinct(
                        (CollectionHandle*)_handle,
                        fieldPtr,
                        filterPtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        if (!string.IsNullOrEmpty(error))
                            throw new IronBaseQueryException(error);
                        return new List<TValue>();
                    }

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (string.IsNullOrEmpty(json))
                        return new List<TValue>();

                    return JsonSerializer.Deserialize<List<TValue>>(json) ?? new List<TValue>();
                }
            }
        }

        // ============== INDEX ==============

        /// <summary>
        /// Create an index.
        /// </summary>
        public string CreateIndex(string field, bool unique = false)
        {
            unsafe
            {
                fixed (byte* fieldPtr = NativeHelper.ToUtf8(field))
                {
                    byte* namePtr;
                    int result = NativeMethods.ironbase_create_index(
                        (CollectionHandle*)_handle,
                        fieldPtr,
                        unique ? 1 : 0,
                        &namePtr
                    );
                    NativeHelper.ThrowIfError(result);
                    return NativeHelper.PtrToStringUtf8AndFree(namePtr) ?? field;
                }
            }
        }

        /// <summary>
        /// Create a compound index.
        /// </summary>
        public string CreateCompoundIndex(IEnumerable<string> fields, bool unique = false)
        {
            var fieldsJson = JsonSerializer.Serialize(fields);

            unsafe
            {
                fixed (byte* fieldsPtr = NativeHelper.ToUtf8(fieldsJson))
                {
                    byte* namePtr;
                    int result = NativeMethods.ironbase_create_compound_index(
                        (CollectionHandle*)_handle,
                        fieldsPtr,
                        unique ? 1 : 0,
                        &namePtr
                    );
                    NativeHelper.ThrowIfError(result);
                    return NativeHelper.PtrToStringUtf8AndFree(namePtr) ?? "";
                }
            }
        }

        /// <summary>
        /// Drop an index.
        /// </summary>
        public void DropIndex(string indexName)
        {
            unsafe
            {
                fixed (byte* namePtr = NativeHelper.ToUtf8(indexName))
                {
                    int result = NativeMethods.ironbase_drop_index(
                        (CollectionHandle*)_handle,
                        namePtr
                    );
                    NativeHelper.ThrowIfError(result);
                }
            }
        }

        /// <summary>
        /// List all indexes.
        /// </summary>
        public List<string> ListIndexes()
        {
            unsafe
            {
                var resultPtr = NativeMethods.ironbase_list_indexes((CollectionHandle*)_handle);
                var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                if (string.IsNullOrEmpty(json))
                    return new List<string>();
                return JsonSerializer.Deserialize<List<string>>(json) ?? new List<string>();
            }
        }

        #region Fuzzy Search

        /// <summary>
        /// Create a fuzzy text index for approximate string matching.
        /// </summary>
        /// <param name="field">Field name to index</param>
        /// <param name="algorithm">Algorithm: "jaro_winkler" (default), "levenshtein", or "damerau_levenshtein"</param>
        /// <param name="threshold">Similarity threshold 0.0-1.0 (default: 0.8)</param>
        /// <returns>Index name</returns>
        public string CreateFuzzyIndex(string field, string algorithm = "jaro_winkler", double threshold = 0.8)
        {
            unsafe
            {
                fixed (byte* fieldPtr = NativeHelper.ToUtf8(field))
                fixed (byte* algoPtr = NativeHelper.ToUtf8(algorithm))
                {
                    byte* namePtr;
                    int result = NativeMethods.ironbase_create_fuzzy_index(
                        (CollectionHandle*)_handle,
                        fieldPtr,
                        algoPtr,
                        threshold,
                        &namePtr
                    );
                    NativeHelper.ThrowIfError(result);
                    return NativeHelper.PtrToStringUtf8AndFree(namePtr) ?? $"{field}_fuzzy";
                }
            }
        }

        /// <summary>
        /// Search for documents using fuzzy text matching.
        /// </summary>
        /// <param name="field">Field to search (must have a fuzzy index)</param>
        /// <param name="query">Search term</param>
        /// <param name="threshold">Override similarity threshold (null to use index default)</param>
        /// <param name="algorithm">Override algorithm (null to use index default)</param>
        /// <returns>List of (document, similarity score) tuples</returns>
        public List<FuzzySearchResult<T>> FuzzySearch(string field, string query, double? threshold = null, string? algorithm = null)
        {
            unsafe
            {
                fixed (byte* fieldPtr = NativeHelper.ToUtf8(field))
                fixed (byte* queryPtr = NativeHelper.ToUtf8(query))
                fixed (byte* algoPtr = algorithm != null ? NativeHelper.ToUtf8(algorithm) : null)
                {
                    var resultPtr = NativeMethods.ironbase_fuzzy_search(
                        (CollectionHandle*)_handle,
                        fieldPtr,
                        queryPtr,
                        threshold ?? -1.0,
                        algoPtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        throw new IronBaseQueryException(error ?? "Fuzzy search failed");
                    }

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (string.IsNullOrEmpty(json))
                        return new List<FuzzySearchResult<T>>();

                    var rawResults = JsonSerializer.Deserialize<List<JsonElement>>(json) ?? new List<JsonElement>();
                    var results = new List<FuzzySearchResult<T>>();

                    foreach (var item in rawResults)
                    {
                        var doc = item.GetProperty("doc");
                        var score = item.GetProperty("score").GetDouble();
                        var document = JsonSerializer.Deserialize<T>(doc.GetRawText());
                        if (document != null)
                        {
                            results.Add(new FuzzySearchResult<T>(document, score));
                        }
                    }

                    return results;
                }
            }
        }

        #endregion

        #region Full-Text Search

        /// <summary>
        /// Create a full-text search index with TF-IDF scoring.
        /// </summary>
        /// <param name="field">Field name to index</param>
        /// <param name="language">Language for stemming: "english" (default), "hungarian", "german", "none"</param>
        /// <param name="minWordLength">Minimum word length to index (null for default: 2)</param>
        /// <param name="accentFolding">Normalize accents (null for default: true)</param>
        /// <returns>Index name</returns>
        public string CreateFulltextIndex(string field, string language = "english", int? minWordLength = null, bool? accentFolding = null)
        {
            unsafe
            {
                fixed (byte* fieldPtr = NativeHelper.ToUtf8(field))
                fixed (byte* langPtr = NativeHelper.ToUtf8(language))
                {
                    byte* namePtr;
                    int result = NativeMethods.ironbase_create_fulltext_index(
                        (CollectionHandle*)_handle,
                        fieldPtr,
                        langPtr,
                        minWordLength ?? 0,
                        accentFolding.HasValue ? (accentFolding.Value ? 1 : 0) : -1,
                        &namePtr
                    );
                    NativeHelper.ThrowIfError(result);
                    return NativeHelper.PtrToStringUtf8AndFree(namePtr) ?? $"{field}_fulltext";
                }
            }
        }

        /// <summary>
        /// Search documents using full-text search with TF-IDF scoring.
        /// </summary>
        /// <param name="field">Field to search (must have a fulltext index)</param>
        /// <param name="query">Search query (words separated by spaces)</param>
        /// <param name="limit">Maximum results (null for default: 10)</param>
        /// <param name="skip">Number of results to skip</param>
        /// <param name="minScore">Minimum TF-IDF score filter</param>
        /// <param name="projection">Fields to include/exclude (e.g., new Dictionary{["title"] = 1})</param>
        /// <returns>List of (document, score, matched tokens) tuples</returns>
        public List<FulltextSearchResult<T>> FulltextSearch(
            string field,
            string query,
            int? limit = null,
            int? skip = null,
            double? minScore = null,
            Dictionary<string, int>? projection = null)
        {
            var projectionJson = projection != null ? JsonSerializer.Serialize(projection) : null;

            unsafe
            {
                fixed (byte* fieldPtr = NativeHelper.ToUtf8(field))
                fixed (byte* queryPtr = NativeHelper.ToUtf8(query))
                fixed (byte* projPtr = projectionJson != null ? NativeHelper.ToUtf8(projectionJson) : null)
                {
                    var resultPtr = NativeMethods.ironbase_fulltext_search(
                        (CollectionHandle*)_handle,
                        fieldPtr,
                        queryPtr,
                        limit ?? 0,
                        skip ?? 0,
                        minScore ?? -1.0,
                        projPtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        throw new IronBaseQueryException(error ?? "Fulltext search failed");
                    }

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (string.IsNullOrEmpty(json))
                        return new List<FulltextSearchResult<T>>();

                    var rawResults = JsonSerializer.Deserialize<List<JsonElement>>(json) ?? new List<JsonElement>();
                    var results = new List<FulltextSearchResult<T>>();

                    foreach (var item in rawResults)
                    {
                        var doc = item.GetProperty("doc");
                        var score = item.GetProperty("score").GetDouble();
                        var tokens = item.GetProperty("tokens").EnumerateArray()
                            .Select(t => t.GetString() ?? "")
                            .ToList();
                        var document = JsonSerializer.Deserialize<T>(doc.GetRawText());
                        if (document != null)
                        {
                            results.Add(new FulltextSearchResult<T>(document, score, tokens));
                        }
                    }

                    return results;
                }
            }
        }

        /// <summary>
        /// List all fulltext indexes for this collection.
        /// </summary>
        /// <returns>List of fulltext index metadata</returns>
        public List<FulltextIndexInfo> ListFulltextIndexes()
        {
            unsafe
            {
                var resultPtr = NativeMethods.ironbase_list_fulltext_indexes((CollectionHandle*)_handle);

                if (resultPtr == null)
                {
                    var error = NativeHelper.GetLastError();
                    throw new IronBaseException(-1, error ?? "Failed to list fulltext indexes");
                }

                var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                if (string.IsNullOrEmpty(json))
                    return new List<FulltextIndexInfo>();

                return JsonSerializer.Deserialize<List<FulltextIndexInfo>>(json) ?? new List<FulltextIndexInfo>();
            }
        }

        #endregion

        /// <summary>
        /// Explain query execution plan.
        /// </summary>
        public string Explain(FilterDefinition<T> filter)
        {
            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filter.ToJson()))
                {
                    var resultPtr = NativeMethods.ironbase_explain(
                        (CollectionHandle*)_handle,
                        filterPtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        throw new IronBaseQueryException(error ?? "Explain failed");
                    }

                    return NativeHelper.PtrToStringUtf8AndFree(resultPtr) ?? "{}";
                }
            }
        }

        // ============== AGGREGATION ==============

        /// <summary>
        /// Execute an aggregation pipeline.
        /// </summary>
        public List<TResult> Aggregate<TResult>(string pipelineJson)
        {
            unsafe
            {
                fixed (byte* pipelinePtr = NativeHelper.ToUtf8(pipelineJson))
                {
                    var resultPtr = NativeMethods.ironbase_aggregate(
                        (CollectionHandle*)_handle,
                        pipelinePtr
                    );

                    if (resultPtr == null)
                    {
                        var error = NativeHelper.GetLastError();
                        if (!string.IsNullOrEmpty(error))
                            throw new IronBaseAggregationException(error);
                        return new List<TResult>();
                    }

                    var json = NativeHelper.PtrToStringUtf8AndFree(resultPtr);
                    if (string.IsNullOrEmpty(json))
                        return new List<TResult>();

                    return JsonSerializer.Deserialize<List<TResult>>(json) ?? new List<TResult>();
                }
            }
        }

        /// <summary>
        /// Execute an aggregation pipeline with BsonDocument stages.
        /// </summary>
        public List<TResult> Aggregate<TResult>(IEnumerable<BsonDocument> pipeline)
        {
            var pipelineJson = JsonSerializer.Serialize(pipeline);
            return Aggregate<TResult>(pipelineJson);
        }

        // ============== CURSOR ==============

        /// <summary>
        /// Create a cursor for streaming through large result sets.
        /// More memory efficient than Find() for large collections.
        /// </summary>
        /// <param name="filter">Query filter (null for all documents)</param>
        /// <param name="batchSize">Number of documents per batch</param>
        /// <returns>Cursor for iterating through results</returns>
        /// <example>
        /// // Process 1 million documents without loading all into memory
        /// using var cursor = collection.FindCursor(null, 500);
        ///
        /// // Iterate one at a time
        /// foreach (var doc in cursor)
        /// {
        ///     Process(doc);
        /// }
        ///
        /// // Or process in batches
        /// cursor.Rewind();
        /// while (!cursor.IsFinished)
        /// {
        ///     var batch = cursor.NextBatch();
        ///     foreach (var doc in batch)
        ///         Process(doc);
        /// }
        /// </example>
        public IronBaseCursor<T> FindCursor(FilterDefinition<T>? filter = null, uint batchSize = 100)
        {
            var filterJson = filter?.ToJson() ?? "{}";

            unsafe
            {
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                {
                    CursorState* cursorPtr;
                    int result = NativeMethods.ironbase_create_cursor(
                        (CollectionHandle*)_handle,
                        filterPtr,
                        batchSize,
                        &cursorPtr
                    );
                    NativeHelper.ThrowIfError(result);
                    return new IronBaseCursor<T>((IntPtr)cursorPtr);
                }
            }
        }

        /// <summary>
        /// Create a cursor with JSON filter string.
        /// </summary>
        public IronBaseCursor<T> FindCursor(string filterJson, uint batchSize = 100)
        {
            return FindCursor(new FilterDefinition<T>(filterJson), batchSize);
        }
    }

    /// <summary>
    /// Options for find operations.
    /// </summary>
    public class FindOptions
    {
        public Dictionary<string, int>? Projection { get; set; }
        public List<(string Field, int Direction)>? Sort { get; set; }
        public int? Limit { get; set; }
        public int? Skip { get; set; }

        /// <summary>
        /// If true, FindWithTotal will also return the total count of matching documents.
        /// Useful for pagination where you need to know total pages.
        /// </summary>
        public bool IncludeTotal { get; set; }

        public string ToJson()
        {
            var options = new Dictionary<string, object?>();

            if (Projection != null)
                options["projection"] = Projection;

            if (Sort != null)
            {
                var sortArray = new List<object[]>();
                foreach (var (field, dir) in Sort)
                    sortArray.Add(new object[] { field, dir });
                options["sort"] = sortArray;
            }

            if (Limit.HasValue)
                options["limit"] = Limit.Value;

            if (Skip.HasValue)
                options["skip"] = Skip.Value;

            return JsonSerializer.Serialize(options);
        }
    }

    /// <summary>
    /// Result of find with total count (for pagination).
    /// </summary>
    public class FindResult<T>
    {
        /// <summary>
        /// Documents matching the query (respecting limit/skip).
        /// </summary>
        public List<T> Documents { get; set; } = new List<T>();

        /// <summary>
        /// Total count of documents matching the filter (ignoring limit/skip).
        /// Only populated when FindOptions.IncludeTotal is true.
        /// </summary>
        public long? Total { get; set; }
    }
}
