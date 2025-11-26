namespace IronBase
{
    /// <summary>
    /// Static factory for creating filter, update, and other definitions.
    /// Similar to MongoDB's Builders class.
    /// </summary>
    /// <typeparam name="T">Document type</typeparam>
    public static class Builders<T>
    {
        private static readonly FilterDefinitionBuilder<T> _filter = new FilterDefinitionBuilder<T>();
        private static readonly UpdateDefinitionBuilder<T> _update = new UpdateDefinitionBuilder<T>();

        /// <summary>
        /// Get the filter builder.
        /// </summary>
        public static FilterDefinitionBuilder<T> Filter => _filter;

        /// <summary>
        /// Get the update builder.
        /// </summary>
        public static UpdateDefinitionBuilder<T> Update => _update;
    }
}
