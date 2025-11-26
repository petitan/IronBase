using System;
using System.IO;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class BasicTests : IDisposable
    {
        private readonly string _testDbPath;
        private IronBaseClient? _client;

        public BasicTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_test_{Guid.NewGuid()}.mlite");
        }

        public void Dispose()
        {
            _client?.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        [Fact]
        public void GetVersion_ReturnsValidVersion()
        {
            var version = IronBaseClient.GetVersion();
            version.Should().NotBeNullOrEmpty();
            version.Should().MatchRegex(@"^\d+\.\d+\.\d+");
        }

        [Fact]
        public void OpenDatabase_CreatesNewFile()
        {
            // Arrange & Act
            _client = new IronBaseClient(_testDbPath);

            // Assert
            File.Exists(_testDbPath).Should().BeTrue();
            _client.Path.Should().Be(_testDbPath);
        }

        [Fact]
        public void ListCollections_EmptyDatabase_ReturnsEmpty()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);

            // Act
            var collections = _client.ListCollections();

            // Assert
            collections.Should().BeEmpty();
        }

        [Fact]
        public void InsertOne_InsertsDocument()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TestUser>("users");

            // Act
            var result = users.InsertOne(new TestUser { Name = "Alice", Age = 30 });

            // Assert
            result.Acknowledged.Should().BeTrue();
            result.InsertedId.Should().NotBeNullOrEmpty();
        }

        [Fact]
        public void FindOne_ReturnsInsertedDocument()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TestUser>("users");
            users.InsertOne(new TestUser { Name = "Bob", Age = 25 });

            // Act
            var filter = Builders<TestUser>.Filter.Eq("Name", "Bob");
            var user = users.FindOne(filter);

            // Assert
            user.Should().NotBeNull();
            user!.Name.Should().Be("Bob");
            user.Age.Should().Be(25);
        }

        [Fact]
        public void CountDocuments_ReturnsCorrectCount()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TestUser>("users");
            users.InsertOne(new TestUser { Name = "Alice", Age = 30 });
            users.InsertOne(new TestUser { Name = "Bob", Age = 25 });
            users.InsertOne(new TestUser { Name = "Charlie", Age = 35 });

            // Act
            var count = users.CountDocuments();

            // Assert
            count.Should().Be(3);
        }

        [Fact]
        public void UpdateOne_UpdatesDocument()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TestUser>("users");
            users.InsertOne(new TestUser { Name = "Alice", Age = 30 });

            // Act
            var filter = Builders<TestUser>.Filter.Eq("Name", "Alice");
            var update = Builders<TestUser>.Update.Set("Age", 31);
            var result = users.UpdateOne(filter, update);

            // Assert
            result.MatchedCount.Should().Be(1);
            result.ModifiedCount.Should().Be(1);

            var updated = users.FindOne(filter);
            updated.Should().NotBeNull();
            updated!.Age.Should().Be(31);
        }

        [Fact]
        public void DeleteOne_DeletesDocument()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TestUser>("users");
            users.InsertOne(new TestUser { Name = "Alice", Age = 30 });

            // Act
            var filter = Builders<TestUser>.Filter.Eq("Name", "Alice");
            var result = users.DeleteOne(filter);

            // Assert
            result.DeletedCount.Should().Be(1);
            users.CountDocuments().Should().Be(0);
        }

        [Fact]
        public void CreateIndex_CreatesIndex()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TestUser>("users");

            // Act
            var indexName = users.CreateIndex("Name");

            // Assert
            indexName.Should().NotBeNullOrEmpty();
            var indexes = users.ListIndexes();
            indexes.Should().Contain(i => i.Contains("Name"));
        }

        [Fact]
        public void BsonDocument_DynamicOperations()
        {
            // Arrange
            _client = new IronBaseClient(_testDbPath);
            var collection = _client.GetDatabase().GetCollection("dynamic");

            // Act
            var doc = new BsonDocument()
                .Add("name", "Dynamic Doc")
                .Add("value", 42)
                .Add("nested", new BsonDocument("inner", "value"));

            var result = collection.InsertOne(doc);

            // Assert
            result.Acknowledged.Should().BeTrue();

            var found = collection.FindOne();
            found.Should().NotBeNull();
            found!["name"].Should().Be("Dynamic Doc");
        }
    }

    public class TestUser
    {
        public string? Name { get; set; }
        public int Age { get; set; }
    }
}
