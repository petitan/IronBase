using System;
using System.Collections.Generic;
using System.Text.Json;

namespace IronBase
{
    /// <summary>
    /// Represents an update operation.
    /// </summary>
    public class UpdateDefinition<T>
    {
        private readonly string _json;

        internal UpdateDefinition(string json)
        {
            _json = json;
        }

        /// <summary>
        /// Create an update from a JSON string.
        /// </summary>
        public static implicit operator UpdateDefinition<T>(string json)
        {
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Get the JSON representation.
        /// </summary>
        public string ToJson() => _json;

        public override string ToString() => _json;
    }

    /// <summary>
    /// Builder for creating update definitions.
    /// </summary>
    public class UpdateDefinitionBuilder<T>
    {
        /// <summary>
        /// Set a field value.
        /// </summary>
        public UpdateDefinition<T> Set(string field, object? value)
        {
            var json = $"{{\"$set\": {{\"{field}\": {JsonSerializer.Serialize(value)}}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Set multiple field values.
        /// </summary>
        public UpdateDefinition<T> Set(IDictionary<string, object?> fields)
        {
            var json = $"{{\"$set\": {JsonSerializer.Serialize(fields)}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Unset (remove) a field.
        /// </summary>
        public UpdateDefinition<T> Unset(string field)
        {
            var json = $"{{\"$unset\": {{\"{field}\": \"\"}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Increment a numeric field.
        /// </summary>
        public UpdateDefinition<T> Inc(string field, long amount)
        {
            var json = $"{{\"$inc\": {{\"{field}\": {amount}}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Increment a numeric field (double).
        /// </summary>
        public UpdateDefinition<T> Inc(string field, double amount)
        {
            var json = $"{{\"$inc\": {{\"{field}\": {JsonSerializer.Serialize(amount)}}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Push a value to an array field.
        /// </summary>
        public UpdateDefinition<T> Push(string field, object value)
        {
            var json = $"{{\"$push\": {{\"{field}\": {JsonSerializer.Serialize(value)}}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Pull (remove) a value from an array field.
        /// </summary>
        public UpdateDefinition<T> Pull(string field, object value)
        {
            var json = $"{{\"$pull\": {{\"{field}\": {JsonSerializer.Serialize(value)}}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Add a value to an array only if it doesn't exist.
        /// </summary>
        public UpdateDefinition<T> AddToSet(string field, object value)
        {
            var json = $"{{\"$addToSet\": {{\"{field}\": {JsonSerializer.Serialize(value)}}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Pop first or last element from an array.
        /// </summary>
        /// <param name="field">Field name</param>
        /// <param name="position">1 = last, -1 = first</param>
        public UpdateDefinition<T> Pop(string field, int position)
        {
            var json = $"{{\"$pop\": {{\"{field}\": {position}}}}}";
            return new UpdateDefinition<T>(json);
        }

        /// <summary>
        /// Combine multiple update operations.
        /// </summary>
        public UpdateDefinition<T> Combine(params UpdateDefinition<T>[] updates)
        {
            // Simple combination - merge $set, $inc, etc.
            var combined = new Dictionary<string, Dictionary<string, object?>>();

            foreach (var update in updates)
            {
                var doc = JsonSerializer.Deserialize<Dictionary<string, JsonElement>>(update.ToJson());
                if (doc == null) continue;

                foreach (var kvp in doc)
                {
                    if (!combined.ContainsKey(kvp.Key))
                        combined[kvp.Key] = new Dictionary<string, object?>();

                    var inner = JsonSerializer.Deserialize<Dictionary<string, object?>>(kvp.Value.GetRawText());
                    if (inner != null)
                    {
                        foreach (var field in inner)
                            combined[kvp.Key][field.Key] = field.Value;
                    }
                }
            }

            return new UpdateDefinition<T>(JsonSerializer.Serialize(combined));
        }
    }
}
