using System;
using System.Collections.Generic;

namespace IronBase
{
    /// <summary>
    /// Represents an IronBase database.
    /// Provides access to collections.
    /// </summary>
    public sealed class IronBaseDatabase
    {
        private readonly IronBaseClient _client;

        internal IronBaseDatabase(IronBaseClient client)
        {
            _client = client ?? throw new ArgumentNullException(nameof(client));
        }

        /// <summary>
        /// Get or create a typed collection.
        /// </summary>
        /// <typeparam name="T">Document type</typeparam>
        /// <param name="name">Collection name</param>
        public IronBaseCollection<T> GetCollection<T>(string name) where T : class
        {
            if (string.IsNullOrEmpty(name))
                throw new ArgumentNullException(nameof(name));

            return new IronBaseCollection<T>(_client, name);
        }

        /// <summary>
        /// Get or create a dynamic collection (BsonDocument).
        /// </summary>
        /// <param name="name">Collection name</param>
        public IronBaseCollection<BsonDocument> GetCollection(string name)
        {
            return GetCollection<BsonDocument>(name);
        }

        /// <summary>
        /// List all collections.
        /// </summary>
        public IReadOnlyList<string> ListCollections()
        {
            return _client.ListCollections();
        }

        /// <summary>
        /// Drop a collection.
        /// </summary>
        public void DropCollection(string name)
        {
            _client.DropCollection(name);
        }

        /// <summary>
        /// Get the parent client.
        /// </summary>
        public IronBaseClient Client => _client;
    }
}
