using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text.Json;
using IronBase.Interop;

namespace IronBase
{
    /// <summary>
    /// Durability mode for database operations.
    /// </summary>
    public enum DurabilityMode
    {
        /// <summary>
        /// Every operation is immediately committed (safest, slowest).
        /// </summary>
        Safe = 0,

        /// <summary>
        /// Operations are batched and committed periodically.
        /// </summary>
        Batch = 1,

        /// <summary>
        /// No automatic commits (fastest, requires manual checkpoint).
        /// </summary>
        Unsafe = 2
    }

    /// <summary>
    /// Main entry point for IronBase.
    /// Represents a connection to an IronBase database file.
    /// </summary>
    public sealed class IronBaseClient : IDisposable
    {
        private IntPtr _handle;
        private bool _disposed;
        private readonly string _path;

        /// <summary>
        /// Open or create a database at the specified path.
        /// </summary>
        /// <param name="path">Path to the database file (.mlite)</param>
        public IronBaseClient(string path) : this(path, DurabilityMode.Safe, 100)
        {
        }

        /// <summary>
        /// Open or create a database with specific durability settings.
        /// </summary>
        /// <param name="path">Path to the database file (.mlite)</param>
        /// <param name="durability">Durability mode</param>
        /// <param name="batchSize">Batch size for Batch mode (ignored for other modes)</param>
        public IronBaseClient(string path, DurabilityMode durability, uint batchSize = 100)
        {
            _path = path ?? throw new ArgumentNullException(nameof(path));

            unsafe
            {
                fixed (byte* pathPtr = NativeHelper.ToUtf8(path))
                {
                    DatabaseHandle* handlePtr;
                    int result = NativeMethods.ironbase_open_with_durability(
                        pathPtr,
                        (int)durability,
                        batchSize,
                        &handlePtr
                    );
                    NativeHelper.ThrowIfError(result);
                    _handle = (IntPtr)handlePtr;
                }
            }
        }

        /// <summary>
        /// Get the database path.
        /// </summary>
        public string Path => _path;

        /// <summary>
        /// Get the default database.
        /// </summary>
        public IronBaseDatabase GetDatabase()
        {
            ThrowIfDisposed();
            return new IronBaseDatabase(this);
        }

        /// <summary>
        /// Get a collection directly (convenience method).
        /// </summary>
        /// <typeparam name="T">Document type</typeparam>
        /// <param name="name">Collection name</param>
        public IronBaseCollection<T> GetCollection<T>(string name) where T : class
        {
            return GetDatabase().GetCollection<T>(name);
        }

        /// <summary>
        /// List all collections.
        /// </summary>
        public IReadOnlyList<string> ListCollections()
        {
            ThrowIfDisposed();

            unsafe
            {
                var jsonPtr = NativeMethods.ironbase_list_collections((DatabaseHandle*)_handle);
                var json = NativeHelper.PtrToStringUtf8AndFree(jsonPtr);

                if (string.IsNullOrEmpty(json))
                    return Array.Empty<string>();

                return JsonSerializer.Deserialize<List<string>>(json) ?? new List<string>();
            }
        }

        /// <summary>
        /// Drop a collection.
        /// </summary>
        /// <param name="name">Collection name</param>
        public void DropCollection(string name)
        {
            ThrowIfDisposed();

            unsafe
            {
                fixed (byte* namePtr = NativeHelper.ToUtf8(name))
                {
                    int result = NativeMethods.ironbase_drop_collection((DatabaseHandle*)_handle, namePtr);
                    NativeHelper.ThrowIfError(result);
                }
            }
        }

        /// <summary>
        /// Flush all pending data to disk.
        /// </summary>
        public void Flush()
        {
            ThrowIfDisposed();

            unsafe
            {
                int result = NativeMethods.ironbase_flush((DatabaseHandle*)_handle);
                NativeHelper.ThrowIfError(result);
            }
        }

        /// <summary>
        /// Checkpoint (clear WAL without flushing metadata).
        /// </summary>
        public void Checkpoint()
        {
            ThrowIfDisposed();

            unsafe
            {
                int result = NativeMethods.ironbase_checkpoint((DatabaseHandle*)_handle);
                NativeHelper.ThrowIfError(result);
            }
        }

        /// <summary>
        /// Get database statistics.
        /// </summary>
        public string GetStats()
        {
            ThrowIfDisposed();

            unsafe
            {
                var jsonPtr = NativeMethods.ironbase_stats((DatabaseHandle*)_handle);
                return NativeHelper.PtrToStringUtf8AndFree(jsonPtr) ?? "{}";
            }
        }

        /// <summary>
        /// Compact the database (remove tombstones).
        /// </summary>
        public CompactionResult Compact()
        {
            ThrowIfDisposed();

            unsafe
            {
                byte* statsPtr;
                int result = NativeMethods.ironbase_compact((DatabaseHandle*)_handle, &statsPtr);
                NativeHelper.ThrowIfError(result);

                var json = NativeHelper.PtrToStringUtf8AndFree(statsPtr) ?? "{}";
                return JsonSerializer.Deserialize<CompactionResult>(json) ?? new CompactionResult();
            }
        }

        /// <summary>
        /// Begin a new transaction.
        /// </summary>
        public ulong BeginTransaction()
        {
            ThrowIfDisposed();

            unsafe
            {
                ulong txId;
                int result = NativeMethods.ironbase_begin_transaction((DatabaseHandle*)_handle, &txId);
                NativeHelper.ThrowIfError(result);
                return txId;
            }
        }

        /// <summary>
        /// Commit a transaction.
        /// </summary>
        public void CommitTransaction(ulong txId)
        {
            ThrowIfDisposed();

            unsafe
            {
                int result = NativeMethods.ironbase_commit((DatabaseHandle*)_handle, txId);
                NativeHelper.ThrowIfError(result);
            }
        }

        /// <summary>
        /// Rollback a transaction.
        /// </summary>
        public void RollbackTransaction(ulong txId)
        {
            ThrowIfDisposed();

            unsafe
            {
                int result = NativeMethods.ironbase_rollback((DatabaseHandle*)_handle, txId);
                NativeHelper.ThrowIfError(result);
            }
        }

        // ============== COLLECTION-LEVEL TRANSACTION OPERATIONS ==============

        /// <summary>
        /// Insert one document within a transaction.
        /// </summary>
        /// <param name="collectionName">Collection name</param>
        /// <param name="document">Document to insert</param>
        /// <param name="txId">Transaction ID from BeginTransaction()</param>
        /// <returns>Insert result with the inserted ID</returns>
        /// <example>
        /// var txId = client.BeginTransaction();
        /// try
        /// {
        ///     client.InsertOneTx("users", new User { Name = "Alice" }, txId);
        ///     client.InsertOneTx("logs", new Log { Action = "created user" }, txId);
        ///     client.CommitTransaction(txId);
        /// }
        /// catch
        /// {
        ///     client.RollbackTransaction(txId);
        ///     throw;
        /// }
        /// </example>
        public InsertOneResult InsertOneTx<T>(string collectionName, T document, ulong txId) where T : class
        {
            ThrowIfDisposed();

            var json = JsonSerializer.Serialize(document);

            unsafe
            {
                fixed (byte* collPtr = NativeHelper.ToUtf8(collectionName))
                fixed (byte* docPtr = NativeHelper.ToUtf8(json))
                {
                    byte* idPtr;
                    int result = NativeMethods.ironbase_insert_one_tx(
                        (DatabaseHandle*)_handle,
                        collPtr,
                        docPtr,
                        txId,
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
        /// Update one document within a transaction.
        /// </summary>
        /// <param name="collectionName">Collection name</param>
        /// <param name="filterJson">Query filter as JSON</param>
        /// <param name="newDocJson">New document content (full replacement)</param>
        /// <param name="txId">Transaction ID</param>
        public UpdateResult UpdateOneTx(string collectionName, string filterJson, string newDocJson, ulong txId)
        {
            ThrowIfDisposed();

            unsafe
            {
                fixed (byte* collPtr = NativeHelper.ToUtf8(collectionName))
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                fixed (byte* newDocPtr = NativeHelper.ToUtf8(newDocJson))
                {
                    ulong matched, modified;
                    int result = NativeMethods.ironbase_update_one_tx(
                        (DatabaseHandle*)_handle,
                        collPtr,
                        filterPtr,
                        newDocPtr,
                        txId,
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
        /// Delete one document within a transaction.
        /// </summary>
        /// <param name="collectionName">Collection name</param>
        /// <param name="filterJson">Query filter as JSON</param>
        /// <param name="txId">Transaction ID</param>
        public DeleteResult DeleteOneTx(string collectionName, string filterJson, ulong txId)
        {
            ThrowIfDisposed();

            unsafe
            {
                fixed (byte* collPtr = NativeHelper.ToUtf8(collectionName))
                fixed (byte* filterPtr = NativeHelper.ToUtf8(filterJson))
                {
                    ulong deleted;
                    int result = NativeMethods.ironbase_delete_one_tx(
                        (DatabaseHandle*)_handle,
                        collPtr,
                        filterPtr,
                        txId,
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

        // ============== SCHEMA VALIDATION ==============

        /// <summary>
        /// Set or clear JSON schema for a collection.
        /// Documents that don't match the schema will be rejected on insert/update.
        /// </summary>
        /// <param name="collectionName">Collection name</param>
        /// <param name="schemaJson">JSON schema definition (null to clear)</param>
        /// <example>
        /// // Set schema for users collection
        /// client.SetCollectionSchema("users", @"{
        ///     ""type"": ""object"",
        ///     ""properties"": {
        ///         ""name"": {""type"": ""string""},
        ///         ""age"": {""type"": ""integer"", ""minimum"": 0}
        ///     },
        ///     ""required"": [""name""]
        /// }");
        ///
        /// // Clear schema
        /// client.SetCollectionSchema("users", null);
        /// </example>
        public void SetCollectionSchema(string collectionName, string? schemaJson)
        {
            ThrowIfDisposed();

            unsafe
            {
                fixed (byte* collPtr = NativeHelper.ToUtf8(collectionName))
                {
                    if (schemaJson == null)
                    {
                        int result = NativeMethods.ironbase_set_collection_schema(
                            (DatabaseHandle*)_handle,
                            collPtr,
                            null
                        );
                        NativeHelper.ThrowIfError(result);
                    }
                    else
                    {
                        fixed (byte* schemaPtr = NativeHelper.ToUtf8(schemaJson))
                        {
                            int result = NativeMethods.ironbase_set_collection_schema(
                                (DatabaseHandle*)_handle,
                                collPtr,
                                schemaPtr
                            );
                            NativeHelper.ThrowIfError(result);
                        }
                    }
                }
            }
        }

        // ============== LOGGING ==============

        /// <summary>
        /// Set the global log level.
        /// </summary>
        /// <param name="level">One of: ERROR, WARN, INFO, DEBUG, TRACE</param>
        /// <example>
        /// IronBaseClient.SetLogLevel("DEBUG");  // Enable debug logging
        /// IronBaseClient.SetLogLevel("WARN");   // Default level
        /// </example>
        public static void SetLogLevel(string level)
        {
            unsafe
            {
                fixed (byte* levelPtr = NativeHelper.ToUtf8(level))
                {
                    int result = NativeMethods.ironbase_set_log_level(levelPtr);
                    NativeHelper.ThrowIfError(result);
                }
            }
        }

        /// <summary>
        /// Get the current global log level.
        /// </summary>
        /// <returns>Current log level (ERROR, WARN, INFO, DEBUG, or TRACE)</returns>
        public static string GetLogLevel()
        {
            unsafe
            {
                var levelPtr = NativeMethods.ironbase_get_log_level();
                return NativeHelper.PtrToStringUtf8AndFree(levelPtr) ?? "WARN";
            }
        }

        /// <summary>
        /// Get the native library version.
        /// </summary>
        public static string GetVersion()
        {
            unsafe
            {
                var versionPtr = NativeMethods.ironbase_version();
                return NativeHelper.PtrToStringUtf8AndFree(versionPtr) ?? "unknown";
            }
        }

        internal IntPtr Handle
        {
            get
            {
                ThrowIfDisposed();
                return _handle;
            }
        }

        private void ThrowIfDisposed()
        {
            if (_disposed)
                throw new ObjectDisposedException(nameof(IronBaseClient));
        }

        public void Dispose()
        {
            if (!_disposed)
            {
                _disposed = true;
                if (_handle != IntPtr.Zero)
                {
                    unsafe
                    {
                        NativeMethods.ironbase_close((DatabaseHandle*)_handle);
                    }
                    _handle = IntPtr.Zero;
                }
            }
        }
    }

    /// <summary>
    /// Result of a compaction operation.
    /// </summary>
    public class CompactionResult
    {
        public ulong SizeBefore { get; set; }
        public ulong SizeAfter { get; set; }
        public ulong SpaceSaved { get; set; }
        public ulong DocumentsScanned { get; set; }
        public ulong DocumentsKept { get; set; }
        public ulong TombstonesRemoved { get; set; }
        public ulong PeakMemoryMb { get; set; }
        public double CompressionRatio { get; set; }
    }
}
