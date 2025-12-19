using System.Collections.Generic;
using System.Text.Json.Serialization;

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

    /// <summary>
    /// Result of a fuzzy search operation.
    /// </summary>
    /// <typeparam name="T">The document type.</typeparam>
    public class FuzzySearchResult<T>
    {
        /// <summary>
        /// The matched document.
        /// </summary>
        public T Document { get; }

        /// <summary>
        /// The similarity score (0.0 to 1.0).
        /// </summary>
        public double Score { get; }

        public FuzzySearchResult(T document, double score)
        {
            Document = document;
            Score = score;
        }
    }

    /// <summary>
    /// Result of a fulltext search operation.
    /// </summary>
    /// <typeparam name="T">The document type.</typeparam>
    public class FulltextSearchResult<T>
    {
        /// <summary>
        /// The matched document.
        /// </summary>
        public T Document { get; }

        /// <summary>
        /// The TF-IDF relevance score.
        /// </summary>
        public double Score { get; }

        /// <summary>
        /// The tokens that matched the query.
        /// </summary>
        public List<string> MatchedTokens { get; }

        public FulltextSearchResult(T document, double score, List<string> matchedTokens)
        {
            Document = document;
            Score = score;
            MatchedTokens = matchedTokens;
        }
    }

    /// <summary>
    /// Information about a fulltext index.
    /// </summary>
    public class FulltextIndexInfo
    {
        /// <summary>
        /// The name of the index.
        /// </summary>
        [JsonPropertyName("name")]
        public string Name { get; set; } = "";

        /// <summary>
        /// The field that is indexed.
        /// </summary>
        [JsonPropertyName("field")]
        public string Field { get; set; } = "";

        /// <summary>
        /// The language used for stemming and stop words.
        /// </summary>
        [JsonPropertyName("language")]
        public string Language { get; set; } = "";

        /// <summary>
        /// Minimum word length for indexing.
        /// </summary>
        [JsonPropertyName("min_word_length")]
        public int MinWordLength { get; set; }

        /// <summary>
        /// Whether accent folding is enabled.
        /// </summary>
        [JsonPropertyName("accent_folding")]
        public bool AccentFolding { get; set; }

        /// <summary>
        /// Number of documents in the index.
        /// </summary>
        [JsonPropertyName("num_documents")]
        public int DocumentCount { get; set; }

        /// <summary>
        /// Number of unique terms in the index.
        /// </summary>
        [JsonPropertyName("num_tokens")]
        public int TermCount { get; set; }
    }
}
