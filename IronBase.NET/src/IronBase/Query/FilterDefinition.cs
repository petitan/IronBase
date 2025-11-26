using System;
using System.Text.Json;

namespace IronBase
{
    /// <summary>
    /// Represents a query filter.
    /// Can be constructed from JSON or using the Builders API.
    /// </summary>
    public class FilterDefinition<T>
    {
        private readonly string _json;

        internal FilterDefinition(string json)
        {
            _json = json;
        }

        /// <summary>
        /// Create a filter from a JSON string.
        /// </summary>
        public static implicit operator FilterDefinition<T>(string json)
        {
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Create an empty filter (matches all documents).
        /// </summary>
        public static FilterDefinition<T> Empty => new FilterDefinition<T>("{}");

        /// <summary>
        /// Get the JSON representation.
        /// </summary>
        public string ToJson() => _json;

        public override string ToString() => _json;
    }

    /// <summary>
    /// Builder for creating filter definitions.
    /// </summary>
    public class FilterDefinitionBuilder<T>
    {
        /// <summary>
        /// Match all documents.
        /// </summary>
        public FilterDefinition<T> Empty => new FilterDefinition<T>("{}");

        /// <summary>
        /// Equality comparison.
        /// </summary>
        public FilterDefinition<T> Eq(string field, object? value)
        {
            var json = $"{{\"{field}\": {JsonSerializer.Serialize(value)}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Not equal comparison.
        /// </summary>
        public FilterDefinition<T> Ne(string field, object? value)
        {
            var json = $"{{\"{field}\": {{\"$ne\": {JsonSerializer.Serialize(value)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Greater than comparison.
        /// </summary>
        public FilterDefinition<T> Gt(string field, object value)
        {
            var json = $"{{\"{field}\": {{\"$gt\": {JsonSerializer.Serialize(value)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Greater than or equal comparison.
        /// </summary>
        public FilterDefinition<T> Gte(string field, object value)
        {
            var json = $"{{\"{field}\": {{\"$gte\": {JsonSerializer.Serialize(value)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Less than comparison.
        /// </summary>
        public FilterDefinition<T> Lt(string field, object value)
        {
            var json = $"{{\"{field}\": {{\"$lt\": {JsonSerializer.Serialize(value)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Less than or equal comparison.
        /// </summary>
        public FilterDefinition<T> Lte(string field, object value)
        {
            var json = $"{{\"{field}\": {{\"$lte\": {JsonSerializer.Serialize(value)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// In array comparison.
        /// </summary>
        public FilterDefinition<T> In(string field, params object[] values)
        {
            var json = $"{{\"{field}\": {{\"$in\": {JsonSerializer.Serialize(values)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Not in array comparison.
        /// </summary>
        public FilterDefinition<T> Nin(string field, params object[] values)
        {
            var json = $"{{\"{field}\": {{\"$nin\": {JsonSerializer.Serialize(values)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Field exists check.
        /// </summary>
        public FilterDefinition<T> Exists(string field, bool exists = true)
        {
            var json = $"{{\"{field}\": {{\"$exists\": {exists.ToString().ToLower()}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Regex match.
        /// </summary>
        public FilterDefinition<T> Regex(string field, string pattern)
        {
            var json = $"{{\"{field}\": {{\"$regex\": {JsonSerializer.Serialize(pattern)}}}}}";
            return new FilterDefinition<T>(json);
        }

        /// <summary>
        /// Logical AND.
        /// </summary>
        public FilterDefinition<T> And(params FilterDefinition<T>[] filters)
        {
            var filtersJson = string.Join(",", Array.ConvertAll(filters, f => f.ToJson()));
            return new FilterDefinition<T>($"{{\"$and\": [{filtersJson}]}}");
        }

        /// <summary>
        /// Logical OR.
        /// </summary>
        public FilterDefinition<T> Or(params FilterDefinition<T>[] filters)
        {
            var filtersJson = string.Join(",", Array.ConvertAll(filters, f => f.ToJson()));
            return new FilterDefinition<T>($"{{\"$or\": [{filtersJson}]}}");
        }

        /// <summary>
        /// Logical NOT.
        /// </summary>
        public FilterDefinition<T> Not(FilterDefinition<T> filter)
        {
            return new FilterDefinition<T>($"{{\"$not\": {filter.ToJson()}}}");
        }
    }
}
