using System;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class FuzzyFulltextTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<Article> _articles;

        public FuzzyFulltextTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_fuzzy_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _articles = _client.GetCollection<Article>("articles");
        }

        public void Dispose()
        {
            _client.Dispose();
            try
            {
                if (File.Exists(_testDbPath))
                    File.Delete(_testDbPath);
                if (File.Exists(_testDbPath + ".wal"))
                    File.Delete(_testDbPath + ".wal");
                if (File.Exists(_testDbPath + ".lock"))
                    File.Delete(_testDbPath + ".lock");
            }
            catch { }
        }

        // ============== FUZZY INDEX ==============

        [Fact]
        public void CreateFuzzyIndex_CreatesIndex()
        {
            var indexName = _articles.CreateFuzzyIndex("Title");

            indexName.Should().NotBeNullOrEmpty();
            indexName.Should().Contain("Title");
            indexName.Should().Contain("fuzzy");
        }

        [Fact]
        public void CreateFuzzyIndex_WithAlgorithm()
        {
            var indexName = _articles.CreateFuzzyIndex("Title", "levenshtein", 0.7);

            indexName.Should().NotBeNullOrEmpty();
        }

        [Fact]
        public void FuzzySearch_FindsSimilarStrings()
        {
            // Create index FIRST - fuzzy index must exist before inserting documents
            _articles.CreateFuzzyIndex("Title");

            // Then insert documents - use simple single-word values for fuzzy matching
            _articles.InsertOne(new Article { Title = "Python", Content = "ML basics" });
            _articles.InsertOne(new Article { Title = "Pithon", Content = "Python tips" });  // Typo of Python
            _articles.InsertOne(new Article { Title = "Java", Content = "Statistics" });

            // Search for similar titles - "Python" should match "Python" (exact) and "Pithon" (typo)
            var results = _articles.FuzzySearch("Title", "Python", threshold: 0.7);

            results.Should().NotBeEmpty();
            results.All(r => r.Score > 0).Should().BeTrue();
        }

        [Fact]
        public void FuzzySearch_WithThreshold()
        {
            // Create index first
            _articles.CreateFuzzyIndex("Title");

            // Then insert documents
            _articles.InsertOne(new Article { Title = "Hello World", Content = "Test" });
            _articles.InsertOne(new Article { Title = "Hallo World", Content = "Test" });
            _articles.InsertOne(new Article { Title = "Goodbye World", Content = "Test" });

            // High threshold - only very similar matches
            var highThreshold = _articles.FuzzySearch("Title", "Hello", threshold: 0.9);

            // Lower threshold - more matches
            var lowThreshold = _articles.FuzzySearch("Title", "Hello", threshold: 0.5);

            lowThreshold.Count.Should().BeGreaterOrEqualTo(highThreshold.Count);
        }

        // ============== FULLTEXT INDEX ==============

        [Fact]
        public void CreateFulltextIndex_CreatesIndex()
        {
            var indexName = _articles.CreateFulltextIndex("Content");

            indexName.Should().NotBeNullOrEmpty();
            indexName.Should().Contain("Content");
            indexName.Should().Contain("fts"); // Full-text search index uses "fts" suffix
        }

        [Fact]
        public void CreateFulltextIndex_WithLanguage()
        {
            var indexName = _articles.CreateFulltextIndex("Content", "hungarian");

            indexName.Should().NotBeNullOrEmpty();
        }

        [Fact]
        public void FulltextSearch_FindsMatchingDocuments()
        {
            // Create index first
            _articles.CreateFulltextIndex("Content", "english");

            // Then insert documents
            _articles.InsertOne(new Article
            {
                Title = "Article 1",
                Content = "The quick brown fox jumps over the lazy dog"
            });
            _articles.InsertOne(new Article
            {
                Title = "Article 2",
                Content = "Python is a programming language"
            });
            _articles.InsertOne(new Article
            {
                Title = "Article 3",
                Content = "Machine learning uses algorithms and data"
            });

            // Search
            var results = _articles.FulltextSearch("Content", "programming");

            results.Should().NotBeEmpty();
            results.Should().Contain(r => r.Document.Content!.Contains("programming"));
            results.All(r => r.Score > 0).Should().BeTrue();
        }

        [Fact]
        public void FulltextSearch_ReturnsMatchedTokens()
        {
            // Create index first
            _articles.CreateFulltextIndex("Content", "english");

            _articles.InsertOne(new Article
            {
                Title = "Test",
                Content = "Learning machine learning is fun and educational"
            });

            var results = _articles.FulltextSearch("Content", "learning");

            results.Should().NotBeEmpty();
            results.First().MatchedTokens.Should().NotBeEmpty();
        }

        [Fact]
        public void FulltextSearch_WithPagination()
        {
            // Create index first
            _articles.CreateFulltextIndex("Content", "english");

            // Insert multiple matching documents
            for (int i = 0; i < 10; i++)
            {
                _articles.InsertOne(new Article
                {
                    Title = $"Article {i}",
                    Content = $"This is document number {i} about programming"
                });
            }

            // Get first 3
            var page1 = _articles.FulltextSearch("Content", "programming", limit: 3);

            // Get next 3
            var page2 = _articles.FulltextSearch("Content", "programming", limit: 3, skip: 3);

            page1.Should().HaveCount(3);
            page2.Should().HaveCount(3);
        }

        [Fact]
        public void ListFulltextIndexes_ReturnsIndexInfo()
        {
            _articles.CreateFulltextIndex("Title", "english");
            _articles.CreateFulltextIndex("Content", "hungarian");

            var indexes = _articles.ListFulltextIndexes();

            indexes.Should().HaveCountGreaterOrEqualTo(2);
            indexes.Should().Contain(i => i.Field == "Title");
            indexes.Should().Contain(i => i.Field == "Content");
        }

        // ============== COMBINED TESTS ==============

        [Fact]
        public void FuzzyAndFulltext_CanCoexist()
        {
            // Create both types of indexes first
            _articles.CreateFuzzyIndex("Title");
            _articles.CreateFulltextIndex("Content", "english");

            // Then insert data
            _articles.InsertOne(new Article
            {
                Title = "Machine Learning Basics",
                Content = "Introduction to ML algorithms and neural networks"
            });

            // Both searches should work
            var fuzzyResults = _articles.FuzzySearch("Title", "Machine");
            var fulltextResults = _articles.FulltextSearch("Content", "algorithms");

            fuzzyResults.Should().NotBeEmpty();
            fulltextResults.Should().NotBeEmpty();
        }
    }

    public class Article
    {
        public string? Title { get; set; }
        public string? Content { get; set; }
    }
}
