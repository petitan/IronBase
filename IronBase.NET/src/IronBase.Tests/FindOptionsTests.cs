using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class FindOptionsTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<Item> _items;

        public FindOptionsTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_options_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _items = _client.GetCollection<Item>("items");
            SeedData();
        }

        private void SeedData()
        {
            _items.InsertOne(new Item { Name = "E", Value = 5, Category = "A" });
            _items.InsertOne(new Item { Name = "B", Value = 2, Category = "B" });
            _items.InsertOne(new Item { Name = "D", Value = 4, Category = "A" });
            _items.InsertOne(new Item { Name = "A", Value = 1, Category = "C" });
            _items.InsertOne(new Item { Name = "C", Value = 3, Category = "B" });
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== SORT ==============

        [Fact]
        public void Sort_Ascending()
        {
            var filter = Builders<Item>.Filter.Empty;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Name", 1) }
            };

            var results = _items.Find(filter, options);

            results.Should().HaveCount(5);
            results[0].Name.Should().Be("A");
            results[1].Name.Should().Be("B");
            results[4].Name.Should().Be("E");
        }

        [Fact]
        public void Sort_Descending()
        {
            var filter = Builders<Item>.Filter.Empty;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Value", -1) }
            };

            var results = _items.Find(filter, options);

            results.Should().HaveCount(5);
            results[0].Value.Should().Be(5);
            results[1].Value.Should().Be(4);
            results[4].Value.Should().Be(1);
        }

        [Fact]
        public void Sort_MultipleFields()
        {
            var filter = Builders<Item>.Filter.Empty;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)>
                {
                    ("Category", 1),
                    ("Value", -1)
                }
            };

            var results = _items.Find(filter, options);

            // Category A: E(5), D(4)
            // Category B: C(3), B(2)
            // Category C: A(1)
            results[0].Category.Should().Be("A");
            results[0].Value.Should().Be(5);
            results[1].Category.Should().Be("A");
            results[1].Value.Should().Be(4);
        }

        // ============== LIMIT ==============

        [Fact]
        public void Limit_ReturnsLimitedResults()
        {
            var filter = Builders<Item>.Filter.Empty;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Value", 1) },
                Limit = 3
            };

            var results = _items.Find(filter, options);

            results.Should().HaveCount(3);
            results.Select(i => i.Value).Should().BeEquivalentTo(new[] { 1, 2, 3 });
        }

        // ============== SKIP ==============

        [Fact]
        public void Skip_SkipsResults()
        {
            var filter = Builders<Item>.Filter.Empty;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Value", 1) },
                Skip = 2
            };

            var results = _items.Find(filter, options);

            results.Should().HaveCount(3);
            results.Select(i => i.Value).Should().BeEquivalentTo(new[] { 3, 4, 5 });
        }

        // ============== SKIP + LIMIT (PAGINATION) ==============

        [Fact]
        public void Pagination_Page1()
        {
            var filter = Builders<Item>.Filter.Empty;
            var pageSize = 2;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Value", 1) },
                Skip = 0,
                Limit = pageSize
            };

            var results = _items.Find(filter, options);

            results.Should().HaveCount(2);
            results.Select(i => i.Value).Should().BeEquivalentTo(new[] { 1, 2 });
        }

        [Fact]
        public void Pagination_Page2()
        {
            var filter = Builders<Item>.Filter.Empty;
            var pageSize = 2;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Value", 1) },
                Skip = pageSize,
                Limit = pageSize
            };

            var results = _items.Find(filter, options);

            results.Should().HaveCount(2);
            results.Select(i => i.Value).Should().BeEquivalentTo(new[] { 3, 4 });
        }

        [Fact]
        public void Pagination_LastPage()
        {
            var filter = Builders<Item>.Filter.Empty;
            var pageSize = 2;
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Value", 1) },
                Skip = 4,
                Limit = pageSize
            };

            var results = _items.Find(filter, options);

            results.Should().HaveCount(1);
            results[0].Value.Should().Be(5);
        }

        // ============== PROJECTION ==============

        [Fact]
        public void Projection_IncludeFields()
        {
            var filter = Builders<Item>.Filter.Empty;
            var options = new FindOptions
            {
                Projection = new Dictionary<string, int>
                {
                    ["Name"] = 1,
                    ["Value"] = 1
                },
                Limit = 1
            };

            var results = _items.Find(filter, options);
            results.Should().HaveCount(1);
            // Note: Excluded fields will have default values
        }

        [Fact]
        public void Projection_ExcludeFields()
        {
            var filter = Builders<Item>.Filter.Empty;
            var options = new FindOptions
            {
                Projection = new Dictionary<string, int>
                {
                    ["Category"] = 0
                },
                Limit = 1
            };

            var results = _items.Find(filter, options);
            results.Should().HaveCount(1);
            // Category should be null/default
        }

        // ============== COMBINED OPTIONS WITH FILTER ==============

        [Fact]
        public void CombinedOptions_FilterSortLimitSkip()
        {
            var filter = Builders<Item>.Filter.In("Category", "A", "B");
            var options = new FindOptions
            {
                Sort = new List<(string Field, int Direction)> { ("Value", -1) },
                Skip = 1,
                Limit = 2
            };

            var results = _items.Find(filter, options);

            // Category A or B: E(5), D(4), C(3), B(2)
            // After Sort desc: E, D, C, B
            // After Skip 1: D, C, B
            // After Limit 2: D, C
            results.Should().HaveCount(2);
            results[0].Value.Should().Be(4); // D
            results[1].Value.Should().Be(3); // C
        }
    }

    public class Item
    {
        public string? Name { get; set; }
        public int Value { get; set; }
        public string? Category { get; set; }
    }
}
