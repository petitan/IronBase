using System;
using System.Collections;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace IronBase
{
    /// <summary>
    /// A dynamic document that can hold any JSON-compatible data.
    /// Similar to MongoDB's BsonDocument.
    /// </summary>
    public class BsonDocument : IDictionary<string, object?>
    {
        private readonly Dictionary<string, object?> _data;

        public BsonDocument()
        {
            _data = new Dictionary<string, object?>();
        }

        public BsonDocument(string key, object? value) : this()
        {
            _data[key] = value;
        }

        public BsonDocument(IDictionary<string, object?> dictionary)
        {
            _data = new Dictionary<string, object?>(dictionary);
        }

        /// <summary>
        /// Get or set a value by key.
        /// </summary>
        public object? this[string key]
        {
            get => _data.TryGetValue(key, out var value) ? value : null;
            set => _data[key] = value;
        }

        /// <summary>
        /// Get a value as a specific type.
        /// </summary>
        public T? GetValue<T>(string key)
        {
            if (_data.TryGetValue(key, out var value))
            {
                if (value is T typedValue)
                    return typedValue;

                if (value is JsonElement element)
                    return JsonSerializer.Deserialize<T>(element.GetRawText());
            }
            return default;
        }

        /// <summary>
        /// Get a nested document.
        /// </summary>
        public BsonDocument? GetDocument(string key)
        {
            var value = this[key];
            if (value is BsonDocument doc)
                return doc;
            if (value is Dictionary<string, object?> dict)
                return new BsonDocument(dict);
            if (value is JsonElement element && element.ValueKind == JsonValueKind.Object)
            {
                var dict2 = JsonSerializer.Deserialize<Dictionary<string, object?>>(element.GetRawText());
                return dict2 != null ? new BsonDocument(dict2) : null;
            }
            return null;
        }

        /// <summary>
        /// Add a key-value pair and return this document (for chaining).
        /// </summary>
        public BsonDocument Add(string key, object? value)
        {
            _data[key] = value;
            return this;
        }

        /// <summary>
        /// Check if the document contains a key.
        /// </summary>
        public bool Contains(string key) => _data.ContainsKey(key);

        /// <summary>
        /// Convert to JSON string.
        /// </summary>
        public string ToJson()
        {
            return JsonSerializer.Serialize(_data);
        }

        /// <summary>
        /// Parse a BsonDocument from JSON string.
        /// </summary>
        public static BsonDocument Parse(string json)
        {
            var dict = JsonSerializer.Deserialize<Dictionary<string, object?>>(json);
            return dict != null ? new BsonDocument(dict) : new BsonDocument();
        }

        public override string ToString() => ToJson();

        // IDictionary implementation
        public ICollection<string> Keys => _data.Keys;
        public ICollection<object?> Values => _data.Values;
        public int Count => _data.Count;
        public bool IsReadOnly => false;

        void IDictionary<string, object?>.Add(string key, object? value) => _data.Add(key, value);
        bool IDictionary<string, object?>.ContainsKey(string key) => _data.ContainsKey(key);
        bool IDictionary<string, object?>.Remove(string key) => _data.Remove(key);
        bool IDictionary<string, object?>.TryGetValue(string key, out object? value) => _data.TryGetValue(key, out value);
        void ICollection<KeyValuePair<string, object?>>.Add(KeyValuePair<string, object?> item) => ((ICollection<KeyValuePair<string, object?>>)_data).Add(item);
        void ICollection<KeyValuePair<string, object?>>.Clear() => _data.Clear();
        bool ICollection<KeyValuePair<string, object?>>.Contains(KeyValuePair<string, object?> item) => ((ICollection<KeyValuePair<string, object?>>)_data).Contains(item);
        void ICollection<KeyValuePair<string, object?>>.CopyTo(KeyValuePair<string, object?>[] array, int arrayIndex) => ((ICollection<KeyValuePair<string, object?>>)_data).CopyTo(array, arrayIndex);
        bool ICollection<KeyValuePair<string, object?>>.Remove(KeyValuePair<string, object?> item) => ((ICollection<KeyValuePair<string, object?>>)_data).Remove(item);
        IEnumerator<KeyValuePair<string, object?>> IEnumerable<KeyValuePair<string, object?>>.GetEnumerator() => _data.GetEnumerator();
        IEnumerator IEnumerable.GetEnumerator() => _data.GetEnumerator();
    }
}
