using System;
using System.Collections.Generic;
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

                    return new InsertManyResult
                    {
                        Acknowledged = true,
                        InsertedCount = resultDoc.TryGetProperty("inserted_count", out var count) ? count.GetInt32() : 0,
                        InsertedIds = null // TODO: parse inserted_ids array
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
}
