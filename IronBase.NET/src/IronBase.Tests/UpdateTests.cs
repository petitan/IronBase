using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class UpdateTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<User> _users;

        public UpdateTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_update_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _users = _client.GetCollection<User>("users");
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== $SET ==============

        [Fact]
        public void Set_UpdatesSingleField()
        {
            _users.InsertOne(new User { Name = "Alice", Age = 30, Score = 100 });

            var filter = Builders<User>.Filter.Eq("Name", "Alice");
            var update = Builders<User>.Update.Set("Age", 31);
            var result = _users.UpdateOne(filter, update);

            result.MatchedCount.Should().Be(1);
            result.ModifiedCount.Should().Be(1);

            var user = _users.FindOne(filter);
            user!.Age.Should().Be(31);
        }

        [Fact]
        public void Set_UpdatesMultipleFields()
        {
            _users.InsertOne(new User { Name = "Bob", Age = 25, Score = 50 });

            var filter = Builders<User>.Filter.Eq("Name", "Bob");
            var update = Builders<User>.Update.Set(new Dictionary<string, object?>
            {
                ["Age"] = 26,
                ["Score"] = 75
            });
            var result = _users.UpdateOne(filter, update);

            result.ModifiedCount.Should().Be(1);

            var user = _users.FindOne(filter);
            user!.Age.Should().Be(26);
            user.Score.Should().Be(75);
        }

        // ============== $INC ==============

        [Fact]
        public void Inc_IncrementsInteger()
        {
            _users.InsertOne(new User { Name = "Charlie", Age = 40, Score = 100 });

            var filter = Builders<User>.Filter.Eq("Name", "Charlie");
            var update = Builders<User>.Update.Inc("Score", 10);
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Score.Should().Be(110);
        }

        [Fact]
        public void Inc_DecrementsWithNegativeValue()
        {
            _users.InsertOne(new User { Name = "Dave", Age = 35, Score = 100 });

            var filter = Builders<User>.Filter.Eq("Name", "Dave");
            var update = Builders<User>.Update.Inc("Score", -25);
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Score.Should().Be(75);
        }

        [Fact]
        public void Inc_IncrementsDouble()
        {
            _users.InsertOne(new User { Name = "Eve", Age = 28, Score = 100, Rating = 4.5 });

            var filter = Builders<User>.Filter.Eq("Name", "Eve");
            var update = Builders<User>.Update.Inc("Rating", 0.5);
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Rating.Should().Be(5.0);
        }

        // ============== $UNSET ==============

        [Fact]
        public void Unset_RemovesField()
        {
            _users.InsertOne(new User { Name = "Frank", Age = 45, Score = 200 });

            var filter = Builders<User>.Filter.Eq("Name", "Frank");
            var update = Builders<User>.Update.Unset("Score");
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Score.Should().Be(0); // Default value after unset
        }

        // ============== ARRAY OPERATORS ==============

        [Fact]
        public void Push_AddsToArray()
        {
            _users.InsertOne(new User { Name = "Grace", Age = 30, Tags = new List<string> { "vip" } });

            var filter = Builders<User>.Filter.Eq("Name", "Grace");
            var update = Builders<User>.Update.Push("Tags", "premium");
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Tags.Should().Contain("vip", "premium");
        }

        [Fact]
        public void Pull_RemovesFromArray()
        {
            _users.InsertOne(new User { Name = "Henry", Age = 50, Tags = new List<string> { "a", "b", "c" } });

            var filter = Builders<User>.Filter.Eq("Name", "Henry");
            var update = Builders<User>.Update.Pull("Tags", "b");
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Tags.Should().HaveCount(2);
            user.Tags.Should().NotContain("b");
        }

        [Fact]
        public void AddToSet_AddsUniqueValue()
        {
            _users.InsertOne(new User { Name = "Ivy", Age = 25, Tags = new List<string> { "a" } });

            var filter = Builders<User>.Filter.Eq("Name", "Ivy");

            // Add new unique value
            var update1 = Builders<User>.Update.AddToSet("Tags", "b");
            _users.UpdateOne(filter, update1);

            // Try to add duplicate
            var update2 = Builders<User>.Update.AddToSet("Tags", "a");
            _users.UpdateOne(filter, update2);

            var user = _users.FindOne(filter);
            user!.Tags.Should().HaveCount(2);
            user.Tags.Should().Contain("a", "b");
        }

        [Fact]
        public void Pop_RemovesLastElement()
        {
            _users.InsertOne(new User { Name = "Jack", Age = 30, Tags = new List<string> { "a", "b", "c" } });

            var filter = Builders<User>.Filter.Eq("Name", "Jack");
            var update = Builders<User>.Update.Pop("Tags", 1); // 1 = last element
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Tags.Should().HaveCount(2);
            user.Tags.Should().NotContain("c");
        }

        [Fact]
        public void Pop_RemovesFirstElement()
        {
            _users.InsertOne(new User { Name = "Kate", Age = 35, Tags = new List<string> { "a", "b", "c" } });

            var filter = Builders<User>.Filter.Eq("Name", "Kate");
            var update = Builders<User>.Update.Pop("Tags", -1); // -1 = first element
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Tags.Should().HaveCount(2);
            user.Tags.Should().NotContain("a");
        }

        // ============== UPDATE MANY ==============

        [Fact]
        public void UpdateMany_UpdatesAllMatching()
        {
            _users.InsertOne(new User { Name = "User1", Age = 20, Score = 0 });
            _users.InsertOne(new User { Name = "User2", Age = 20, Score = 0 });
            _users.InsertOne(new User { Name = "User3", Age = 30, Score = 0 });

            var filter = Builders<User>.Filter.Eq("Age", 20);
            var update = Builders<User>.Update.Set("Score", 100);
            var result = _users.UpdateMany(filter, update);

            result.MatchedCount.Should().Be(2);
            result.ModifiedCount.Should().Be(2);

            var updated = _users.Find(filter);
            updated.All(u => u.Score == 100).Should().BeTrue();
        }

        // ============== COMBINED UPDATES ==============

        [Fact]
        public void Combine_MultipleUpdateOperations()
        {
            _users.InsertOne(new User { Name = "Combined", Age = 25, Score = 50 });

            var filter = Builders<User>.Filter.Eq("Name", "Combined");
            var update = Builders<User>.Update.Combine(
                Builders<User>.Update.Set("Age", 26),
                Builders<User>.Update.Inc("Score", 25)
            );
            _users.UpdateOne(filter, update);

            var user = _users.FindOne(filter);
            user!.Age.Should().Be(26);
            user.Score.Should().Be(75);
        }

        // ============== NO MATCH ==============

        [Fact]
        public void UpdateOne_NoMatch_ReturnsZero()
        {
            var filter = Builders<User>.Filter.Eq("Name", "NonExistent");
            var update = Builders<User>.Update.Set("Age", 100);
            var result = _users.UpdateOne(filter, update);

            result.MatchedCount.Should().Be(0);
            result.ModifiedCount.Should().Be(0);
        }
    }

    public class User
    {
        public string? Name { get; set; }
        public int Age { get; set; }
        public int Score { get; set; }
        public double Rating { get; set; }
        public List<string>? Tags { get; set; }
    }
}
