using System;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class QueryTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<Product> _products;

        public QueryTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_query_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _products = _client.GetCollection<Product>("products");
            SeedData();
        }

        private void SeedData()
        {
            _products.InsertOne(new Product { Name = "Apple", Price = 1.50, Category = "Fruit", Stock = 100 });
            _products.InsertOne(new Product { Name = "Banana", Price = 0.75, Category = "Fruit", Stock = 150 });
            _products.InsertOne(new Product { Name = "Milk", Price = 2.50, Category = "Dairy", Stock = 50 });
            _products.InsertOne(new Product { Name = "Cheese", Price = 5.00, Category = "Dairy", Stock = 30 });
            _products.InsertOne(new Product { Name = "Bread", Price = 3.00, Category = "Bakery", Stock = 75 });
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== COMPARISON OPERATORS ==============

        [Fact]
        public void Eq_FindsMatchingDocuments()
        {
            var filter = Builders<Product>.Filter.Eq("Category", "Fruit");
            var results = _products.Find(filter);
            results.Should().HaveCount(2);
            results.All(p => p.Category == "Fruit").Should().BeTrue();
        }

        [Fact]
        public void Ne_ExcludesMatchingDocuments()
        {
            var filter = Builders<Product>.Filter.Ne("Category", "Fruit");
            var results = _products.Find(filter);
            results.Should().HaveCount(3);
            results.Any(p => p.Category == "Fruit").Should().BeFalse();
        }

        [Fact]
        public void Gt_FindsGreaterThan()
        {
            var filter = Builders<Product>.Filter.Gt("Price", 2.0);
            var results = _products.Find(filter);
            results.Should().HaveCount(3);
            results.All(p => p.Price > 2.0).Should().BeTrue();
        }

        [Fact]
        public void Gte_FindsGreaterThanOrEqual()
        {
            var filter = Builders<Product>.Filter.Gte("Price", 2.50);
            var results = _products.Find(filter);
            results.Should().HaveCount(3);
            results.All(p => p.Price >= 2.50).Should().BeTrue();
        }

        [Fact]
        public void Lt_FindsLessThan()
        {
            var filter = Builders<Product>.Filter.Lt("Price", 2.0);
            var results = _products.Find(filter);
            results.Should().HaveCount(2);
            results.All(p => p.Price < 2.0).Should().BeTrue();
        }

        [Fact]
        public void Lte_FindsLessThanOrEqual()
        {
            var filter = Builders<Product>.Filter.Lte("Price", 2.50);
            var results = _products.Find(filter);
            results.Should().HaveCount(3);
            results.All(p => p.Price <= 2.50).Should().BeTrue();
        }

        [Fact]
        public void In_FindsInArray()
        {
            var filter = Builders<Product>.Filter.In("Category", "Fruit", "Dairy");
            var results = _products.Find(filter);
            results.Should().HaveCount(4);
        }

        [Fact]
        public void Nin_ExcludesArray()
        {
            var filter = Builders<Product>.Filter.Nin("Category", "Fruit", "Dairy");
            var results = _products.Find(filter);
            results.Should().HaveCount(1);
            results.First().Category.Should().Be("Bakery");
        }

        // ============== LOGICAL OPERATORS ==============

        [Fact]
        public void And_CombinesFilters()
        {
            var filter = Builders<Product>.Filter.And(
                Builders<Product>.Filter.Eq("Category", "Dairy"),
                Builders<Product>.Filter.Gt("Price", 3.0)
            );
            var results = _products.Find(filter);
            results.Should().HaveCount(1);
            results.First().Name.Should().Be("Cheese");
        }

        [Fact]
        public void Or_MatchesEitherFilter()
        {
            var filter = Builders<Product>.Filter.Or(
                Builders<Product>.Filter.Eq("Name", "Apple"),
                Builders<Product>.Filter.Eq("Name", "Milk")
            );
            var results = _products.Find(filter);
            results.Should().HaveCount(2);
        }

        // ============== ELEMENT OPERATORS ==============

        [Fact]
        public void Exists_ChecksFieldPresence()
        {
            var filter = Builders<Product>.Filter.Exists("Price");
            var results = _products.Find(filter);
            results.Should().HaveCount(5);
        }

        [Fact]
        public void Exists_False_ChecksFieldAbsence()
        {
            var filter = Builders<Product>.Filter.Exists("NonExistentField", false);
            var results = _products.Find(filter);
            results.Should().HaveCount(5);
        }

        // ============== REGEX ==============

        [Fact]
        public void Regex_MatchesPattern()
        {
            // Use simple substring match - Rust regex uses different anchoring
            var filter = Builders<Product>.Filter.Regex("Name", "an");
            var results = _products.Find(filter);
            // "Banana" contains "an"
            results.Should().HaveCountGreaterOrEqualTo(1);
        }

        [Fact]
        public void Regex_MatchesExactPattern()
        {
            // Match exact value with regex
            var filter = Builders<Product>.Filter.Regex("Name", "Apple");
            var results = _products.Find(filter);
            results.Should().HaveCount(1);
            results.First().Name.Should().Be("Apple");
        }

        // ============== JSON FILTER ==============

        [Fact]
        public void Find_WithJsonFilter()
        {
            var results = _products.Find("{\"Category\": \"Bakery\"}");
            results.Should().HaveCount(1);
            results.First().Name.Should().Be("Bread");
        }

        // ============== EMPTY FILTER ==============

        [Fact]
        public void Find_EmptyFilter_ReturnsAll()
        {
            var results = _products.Find();
            results.Should().HaveCount(5);
        }

        [Fact]
        public void Find_ExplicitEmptyFilter_ReturnsAll()
        {
            var filter = Builders<Product>.Filter.Empty;
            var results = _products.Find(filter);
            results.Should().HaveCount(5);
        }

        // ============== COUNT WITH FILTER ==============

        [Fact]
        public void CountDocuments_WithFilter()
        {
            var filter = Builders<Product>.Filter.Eq("Category", "Fruit");
            var count = _products.CountDocuments(filter);
            count.Should().Be(2);
        }

        // ============== DISTINCT ==============

        [Fact]
        public void Distinct_ReturnsUniqueValues()
        {
            var categories = _products.Distinct<string>("Category");
            categories.Should().HaveCount(3);
            categories.Should().Contain("Fruit", "Dairy", "Bakery");
        }

        [Fact]
        public void Distinct_WithFilter()
        {
            var filter = Builders<Product>.Filter.Gt("Price", 2.0);
            var categories = _products.Distinct<string>("Category", filter);
            categories.Should().Contain("Dairy", "Bakery");
        }
    }

    public class Product
    {
        public string? Name { get; set; }
        public double Price { get; set; }
        public string? Category { get; set; }
        public int Stock { get; set; }
    }
}
