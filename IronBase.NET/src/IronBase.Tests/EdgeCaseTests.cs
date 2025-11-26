using System;
using System.IO;
using System.Collections.Generic;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class EdgeCaseTests : IDisposable
    {
        private readonly string _testDbPath;
        private IronBaseClient? _client;

        public EdgeCaseTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_edge_test_{Guid.NewGuid()}.mlite");
        }

        public void Dispose()
        {
            _client?.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== EMPTY RESULTS ==============

        [Fact]
        public void Find_NoMatch_ReturnsEmptyList()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");
            users.InsertOne(new EdgeUser { Name = "Alice" });

            var filter = Builders<EdgeUser>.Filter.Eq("Name", "NonExistent");
            var results = users.Find(filter);

            results.Should().BeEmpty();
        }

        [Fact]
        public void FindOne_NoMatch_ReturnsNull()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            var result = users.FindOne(Builders<EdgeUser>.Filter.Eq("Name", "Ghost"));

            result.Should().BeNull();
        }

        [Fact]
        public void CountDocuments_EmptyCollection_ReturnsZero()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            var count = users.CountDocuments();

            count.Should().Be(0);
        }

        // ============== SPECIAL CHARACTERS ==============

        [Fact]
        public void InsertAndFind_SpecialCharactersInString()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            var specialName = "O'Brien \"The Great\" <test> & Co.";
            users.InsertOne(new EdgeUser { Name = specialName });

            var found = users.FindOne();
            found!.Name.Should().Be(specialName);
        }

        [Fact]
        public void InsertAndFind_UnicodeCharacters()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            var unicodeName = "日本語テスト 中文测试 한국어테스트 🎉🚀";
            users.InsertOne(new EdgeUser { Name = unicodeName });

            var found = users.FindOne();
            found!.Name.Should().Be(unicodeName);
        }

        [Fact]
        public void InsertAndFind_EmptyString()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            users.InsertOne(new EdgeUser { Name = "" });

            var found = users.FindOne();
            found!.Name.Should().BeEmpty();
        }

        // ============== NULL VALUES ==============

        [Fact]
        public void InsertAndFind_NullableField()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            users.InsertOne(new EdgeUser { Name = null, Age = 30 });

            var found = users.FindOne();
            found!.Name.Should().BeNull();
            found.Age.Should().Be(30);
        }

        // ============== LARGE VALUES ==============

        [Fact]
        public void InsertAndFind_LargeString()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            var largeString = new string('X', 10000);
            users.InsertOne(new EdgeUser { Name = largeString });

            var found = users.FindOne();
            found!.Name.Should().HaveLength(10000);
        }

        [Fact]
        public void InsertAndFind_LargeNumber()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            users.InsertOne(new EdgeUser { Name = "BigNum", Age = int.MaxValue });

            var found = users.FindOne();
            found!.Age.Should().Be(int.MaxValue);
        }

        [Fact]
        public void InsertAndFind_NegativeNumber()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            users.InsertOne(new EdgeUser { Name = "NegNum", Age = -12345 });

            var found = users.FindOne();
            found!.Age.Should().Be(-12345);
        }

        // ============== COLLECTION NAMES ==============

        [Fact]
        public void Collection_WithUnderscores()
        {
            _client = new IronBaseClient(_testDbPath);
            var coll = _client.GetCollection<EdgeUser>("my_collection_name");

            coll.InsertOne(new EdgeUser { Name = "Test" });
            coll.CountDocuments().Should().Be(1);
        }

        [Fact]
        public void Collection_WithNumbers()
        {
            _client = new IronBaseClient(_testDbPath);
            var coll = _client.GetCollection<EdgeUser>("collection123");

            coll.InsertOne(new EdgeUser { Name = "Test" });
            coll.CountDocuments().Should().Be(1);
        }

        // ============== DISPOSED OBJECT ==============

        [Fact]
        public void DisposedClient_ThrowsOnUse()
        {
            _client = new IronBaseClient(_testDbPath);
            _client.Dispose();

            Action act = () => _client.ListCollections();
            act.Should().Throw<ObjectDisposedException>();

            _client = null; // Prevent double dispose in Dispose()
        }

        // ============== MULTIPLE OPERATIONS ==============

        [Fact]
        public void RapidInsertDelete_WorksCorrectly()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            for (int i = 0; i < 100; i++)
            {
                users.InsertOne(new EdgeUser { Name = $"User{i}" });
            }

            users.CountDocuments().Should().Be(100);

            for (int i = 0; i < 50; i++)
            {
                users.DeleteOne(Builders<EdgeUser>.Filter.Eq("Name", $"User{i}"));
            }

            users.CountDocuments().Should().Be(50);
        }

        [Fact]
        public void MultipleUpdatesOnSameDocument()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");

            users.InsertOne(new EdgeUser { Name = "MultiUpdate", Age = 0 });

            var filter = Builders<EdgeUser>.Filter.Eq("Name", "MultiUpdate");

            for (int i = 1; i <= 10; i++)
            {
                users.UpdateOne(filter, Builders<EdgeUser>.Update.Inc("Age", 1));
            }

            var found = users.FindOne(filter);
            found!.Age.Should().Be(10);
        }

        // ============== FILTER EDGE CASES ==============

        [Fact]
        public void Filter_EmptyAndFilter()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");
            users.InsertOne(new EdgeUser { Name = "Test", Age = 25 });

            // Empty $and should match all
            var filter = Builders<EdgeUser>.Filter.And();
            var results = users.Find(filter);

            results.Should().HaveCount(1);
        }

        [Fact]
        public void Filter_SingleElementAnd()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");
            users.InsertOne(new EdgeUser { Name = "Single", Age = 30 });
            users.InsertOne(new EdgeUser { Name = "Other", Age = 40 });

            var filter = Builders<EdgeUser>.Filter.And(
                Builders<EdgeUser>.Filter.Eq("Name", "Single")
            );
            var results = users.Find(filter);

            results.Should().HaveCount(1);
            results[0].Name.Should().Be("Single");
        }

        // ============== BOUNDARY VALUES ==============

        [Fact]
        public void Limit_One_ReturnsSingleDocument()
        {
            // Note: Limit = 0 may return all documents in some databases
            // This test verifies Limit = 1 works correctly
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");
            users.InsertOne(new EdgeUser { Name = "Test1" });
            users.InsertOne(new EdgeUser { Name = "Test2" });
            users.InsertOne(new EdgeUser { Name = "Test3" });

            var results = users.Find(Builders<EdgeUser>.Filter.Empty, new FindOptions { Limit = 1 });

            results.Should().HaveCount(1);
        }

        [Fact]
        public void Skip_MoreThanTotal_ReturnsEmpty()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<EdgeUser>("users");
            users.InsertOne(new EdgeUser { Name = "Test" });

            var results = users.Find(Builders<EdgeUser>.Filter.Empty, new FindOptions { Skip = 100 });

            results.Should().BeEmpty();
        }
    }

    public class EdgeUser
    {
        public string? Name { get; set; }
        public int Age { get; set; }
    }
}
