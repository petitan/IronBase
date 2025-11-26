namespace IronBase
{
    /// <summary>
    /// Result of an insert operation.
    /// </summary>
    public class InsertOneResult
    {
        /// <summary>
        /// Whether the operation was acknowledged.
        /// </summary>
        public bool Acknowledged { get; set; } = true;

        /// <summary>
        /// The ID of the inserted document (as JSON string).
        /// </summary>
        public string? InsertedId { get; set; }
    }

    /// <summary>
    /// Result of an insert many operation.
    /// </summary>
    public class InsertManyResult
    {
        /// <summary>
        /// Whether the operation was acknowledged.
        /// </summary>
        public bool Acknowledged { get; set; } = true;

        /// <summary>
        /// Number of documents inserted.
        /// </summary>
        public int InsertedCount { get; set; }

        /// <summary>
        /// IDs of inserted documents (as JSON strings).
        /// </summary>
        public string[]? InsertedIds { get; set; }
    }

    /// <summary>
    /// Result of an update operation.
    /// </summary>
    public class UpdateResult
    {
        /// <summary>
        /// Whether the operation was acknowledged.
        /// </summary>
        public bool Acknowledged { get; set; } = true;

        /// <summary>
        /// Number of documents matched.
        /// </summary>
        public long MatchedCount { get; set; }

        /// <summary>
        /// Number of documents modified.
        /// </summary>
        public long ModifiedCount { get; set; }
    }

    /// <summary>
    /// Result of a delete operation.
    /// </summary>
    public class DeleteResult
    {
        /// <summary>
        /// Whether the operation was acknowledged.
        /// </summary>
        public bool Acknowledged { get; set; } = true;

        /// <summary>
        /// Number of documents deleted.
        /// </summary>
        public long DeletedCount { get; set; }
    }
}
