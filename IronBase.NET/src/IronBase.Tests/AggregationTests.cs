using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class AggregationTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<Order> _orders;

        public AggregationTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_agg_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _orders = _client.GetCollection<Order>("orders");
            SeedData();
        }

        private void SeedData()
        {
            _orders.InsertOne(new Order { Customer = "Alice", Product = "Apple", Quantity = 10, Price = 1.50 });
            _orders.InsertOne(new Order { Customer = "Alice", Product = "Banana", Quantity = 5, Price = 0.75 });
            _orders.InsertOne(new Order { Customer = "Bob", Product = "Apple", Quantity = 20, Price = 1.50 });
            _orders.InsertOne(new Order { Customer = "Bob", Product = "Milk", Quantity = 2, Price = 2.50 });
            _orders.InsertOne(new Order { Customer = "Charlie", Product = "Bread", Quantity = 3, Price = 3.00 });
            _orders.InsertOne(new Order { Customer = "Charlie", Product = "Apple", Quantity = 15, Price = 1.50 });
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== $MATCH ==============

        [Fact]
        public void Match_FiltersByCondition()
        {
            var pipeline = @"[{""$match"": {""Customer"": ""Alice""}}]";
            var results = _orders.Aggregate<Order>(pipeline);

            results.Should().HaveCount(2);
            results.All(o => o.Customer == "Alice").Should().BeTrue();
        }

        // ============== $GROUP ==============

        [Fact]
        public void Group_ByField_WithSum()
        {
            var pipeline = @"[
                {""$group"": {""_id"": ""$Customer"", ""total"": {""$sum"": ""$Quantity""}}}
            ]";
            var results = _orders.Aggregate<AggResult>(pipeline);

            results.Should().HaveCount(3);

            var alice = results.First(r => (r._id?.ToString() ?? "") == "Alice");
            alice.total.Should().Be(15); // 10 + 5

            var bob = results.First(r => (r._id?.ToString() ?? "") == "Bob");
            bob.total.Should().Be(22); // 20 + 2
        }

        [Fact]
        public void Group_WithCount()
        {
            var pipeline = @"[
                {""$group"": {""_id"": ""$Product"", ""count"": {""$sum"": 1}}}
            ]";
            var results = _orders.Aggregate<AggResult>(pipeline);

            var appleCount = results.First(r => (r._id?.ToString() ?? "") == "Apple");
            appleCount.count.Should().Be(3); // Alice, Bob, Charlie ordered Apple
        }

        [Fact]
        public void Group_WithAverage()
        {
            var pipeline = @"[
                {""$group"": {""_id"": ""$Product"", ""avgQty"": {""$avg"": ""$Quantity""}}}
            ]";
            var results = _orders.Aggregate<AggResult>(pipeline);

            var apple = results.First(r => (r._id?.ToString() ?? "") == "Apple");
            apple.avgQty.Should().Be(15); // (10 + 20 + 15) / 3
        }

        [Fact]
        public void Group_WithMinMax()
        {
            var pipeline = @"[
                {""$group"": {""_id"": null, ""minPrice"": {""$min"": ""$Price""}, ""maxPrice"": {""$max"": ""$Price""}}}
            ]";
            var results = _orders.Aggregate<AggResult>(pipeline);

            results.Should().HaveCount(1);
            results[0].minPrice.Should().Be(0.75);
            results[0].maxPrice.Should().Be(3.00);
        }

        // ============== $SORT ==============

        [Fact]
        public void Sort_ByField()
        {
            var pipeline = @"[
                {""$match"": {""Customer"": ""Alice""}},
                {""$sort"": {""Quantity"": -1}}
            ]";
            var results = _orders.Aggregate<Order>(pipeline);

            results.Should().HaveCount(2);
            results[0].Quantity.Should().Be(10);
            results[1].Quantity.Should().Be(5);
        }

        // ============== $LIMIT ==============

        [Fact]
        public void Limit_ReturnsTopN()
        {
            var pipeline = @"[
                {""$sort"": {""Quantity"": -1}},
                {""$limit"": 2}
            ]";
            var results = _orders.Aggregate<Order>(pipeline);

            results.Should().HaveCount(2);
            results[0].Quantity.Should().Be(20); // Bob's Apple order
            results[1].Quantity.Should().Be(15); // Charlie's Apple order
        }

        // ============== $SKIP ==============

        [Fact]
        public void Skip_SkipsTopN()
        {
            var pipeline = @"[
                {""$sort"": {""Quantity"": -1}},
                {""$skip"": 3}
            ]";
            var results = _orders.Aggregate<Order>(pipeline);

            results.Should().HaveCount(3);
            // Should skip the top 3 quantities (20, 15, 10)
        }

        // ============== $PROJECT ==============

        [Fact]
        public void Project_SelectsFields()
        {
            var pipeline = @"[
                {""$match"": {""Customer"": ""Alice""}},
                {""$project"": {""Product"": 1, ""Quantity"": 1}}
            ]";
            var results = _orders.Aggregate<ProjectedOrder>(pipeline);

            results.Should().HaveCount(2);
            results.All(o => !string.IsNullOrEmpty(o.Product)).Should().BeTrue();
        }

        // ============== COMBINED PIPELINE ==============

        [Fact]
        public void CombinedPipeline_MatchGroupSort()
        {
            var pipeline = @"[
                {""$match"": {""Product"": ""Apple""}},
                {""$group"": {""_id"": ""$Customer"", ""totalApples"": {""$sum"": ""$Quantity""}}},
                {""$sort"": {""totalApples"": -1}}
            ]";
            var results = _orders.Aggregate<AggResult>(pipeline);

            results.Should().HaveCount(3);
            // _id may be JsonElement or string, convert to string for comparison
            results[0]._id?.ToString().Should().Be("Bob"); // 20 apples
            results[1]._id?.ToString().Should().Be("Charlie"); // 15 apples
            results[2]._id?.ToString().Should().Be("Alice"); // 10 apples
        }

        // ============== BSONDOCUMENT PIPELINE ==============

        [Fact]
        public void BsonDocumentPipeline_Works()
        {
            var pipeline = new List<BsonDocument>
            {
                new BsonDocument("$match", new BsonDocument("Customer", "Alice")),
                new BsonDocument("$group", new BsonDocument()
                    .Add("_id", "$Customer")
                    .Add("total", new BsonDocument("$sum", "$Quantity")))
            };

            var results = _orders.Aggregate<AggResult>(pipeline);

            results.Should().HaveCount(1);
            results[0].total.Should().Be(15);
        }
    }

    public class Order
    {
        public string? Customer { get; set; }
        public string? Product { get; set; }
        public int Quantity { get; set; }
        public double Price { get; set; }
    }

    public class AggResult
    {
        public object? _id { get; set; }
        public int total { get; set; }
        public int count { get; set; }
        public double avgQty { get; set; }
        public double minPrice { get; set; }
        public double maxPrice { get; set; }
        public int totalApples { get; set; }
    }

    public class ProjectedOrder
    {
        public string? Product { get; set; }
        public int Quantity { get; set; }
    }
}
