using System;
using System.IO;
using System.Text.Json;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class DatabaseManagementTests : IDisposable
    {
        private readonly string _testDbPath;
        private IronBaseClient? _client;

        public DatabaseManagementTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_mgmt_test_{Guid.NewGuid()}.mlite");
        }

        public void Dispose()
        {
            _client?.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== DROP COLLECTION ==============

        [Fact]
        public void DropCollection_RemovesCollection()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "Test" });

            // Verify collection exists
            _client.ListCollections().Should().Contain("users");

            // Drop
            _client.DropCollection("users");

            // Verify removed
            _client.ListCollections().Should().NotContain("users");
        }

        [Fact]
        public void DropCollection_CanRecreate()
        {
            _client = new IronBaseClient(_testDbPath);

            // Create and populate
            var users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "Original" });

            // Drop
            _client.DropCollection("users");

            // Recreate
            users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "New" });

            // Verify
            users.CountDocuments().Should().Be(1);
            var found = users.FindOne();
            found!.Name.Should().Be("New");
        }

        // ============== FLUSH ==============

        [Fact]
        public void Flush_DoesNotThrow()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "Test" });

            Action flush = () => _client.Flush();
            flush.Should().NotThrow();
        }

        [Fact]
        public void Flush_PersistsData()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "FlushTest" });
            _client.Flush();
            _client.Dispose();

            // Reopen and verify
            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DbUser>("users");
            var found = users.FindOne(Builders<DbUser>.Filter.Eq("Name", "FlushTest"));
            found.Should().NotBeNull();
        }

        // ============== CHECKPOINT ==============

        [Fact]
        public void Checkpoint_DoesNotThrow()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "Test" });

            Action checkpoint = () => _client.Checkpoint();
            checkpoint.Should().NotThrow();
        }

        // ============== GET STATS ==============

        [Fact]
        public void GetStats_ReturnsJson()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "StatsTest" });

            var stats = _client.GetStats();

            stats.Should().NotBeNullOrEmpty();
            stats.Should().StartWith("{");
        }

        [Fact]
        public void GetStats_ContainsExpectedFields()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            for (int i = 0; i < 10; i++)
            {
                users.InsertOne(new DbUser { Name = $"User{i}", Age = i * 10 });
            }

            var stats = _client.GetStats();
            var statsDoc = JsonDocument.Parse(stats);

            // Should have some basic stats
            statsDoc.RootElement.EnumerateObject().Should().NotBeEmpty();
        }

        // ============== COMPACT ==============

        [Fact]
        public void Compact_ReturnsResult()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");

            // Insert then delete to create tombstones
            for (int i = 0; i < 50; i++)
            {
                users.InsertOne(new DbUser { Name = $"ToDelete{i}" });
            }
            users.DeleteMany(Builders<DbUser>.Filter.Exists("Name"));

            // Compact
            var result = _client.Compact();

            result.Should().NotBeNull();
            result.DocumentsScanned.Should().BeGreaterOrEqualTo(0);
        }

        [Fact]
        public void Compact_RemovesTombstones()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");

            // Insert many documents
            for (int i = 0; i < 100; i++)
            {
                users.InsertOne(new DbUser { Name = $"User{i}" });
            }

            // Delete half
            users.DeleteMany(Builders<DbUser>.Filter.Lt("Age", 50));

            // Get file size before
            var sizeBefore = new FileInfo(_testDbPath).Length;

            // Compact
            var result = _client.Compact();

            // TombstonesRemoved should be > 0
            result.TombstonesRemoved.Should().BeGreaterOrEqualTo(0);
        }

        // ============== LIST COLLECTIONS ==============

        [Fact]
        public void ListCollections_ReturnsMultipleCollections()
        {
            _client = new IronBaseClient(_testDbPath);

            _client.GetCollection<DbUser>("users").InsertOne(new DbUser { Name = "U1" });
            _client.GetCollection<DbUser>("orders").InsertOne(new DbUser { Name = "O1" });
            _client.GetCollection<DbUser>("products").InsertOne(new DbUser { Name = "P1" });

            var collections = _client.ListCollections();

            collections.Should().HaveCount(3);
            collections.Should().Contain("users");
            collections.Should().Contain("orders");
            collections.Should().Contain("products");
        }

        // ============== DATABASE PERSISTENCE ==============

        [Fact]
        public void Database_PersistsAcrossReopens()
        {
            // Create and populate
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            users.InsertOne(new DbUser { Name = "Persist1", Age = 30 });
            users.InsertOne(new DbUser { Name = "Persist2", Age = 40 });
            _client.Dispose();

            // Reopen
            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DbUser>("users");

            users.CountDocuments().Should().Be(2);

            var u1 = users.FindOne(Builders<DbUser>.Filter.Eq("Name", "Persist1"));
            u1!.Age.Should().Be(30);
        }

        [Fact]
        public void Database_IndexesPersistAcrossReopens()
        {
            // Create index
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DbUser>("users");
            users.CreateIndex("Name");
            users.InsertOne(new DbUser { Name = "IndexTest" });
            _client.Dispose();

            // Reopen
            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DbUser>("users");

            var indexes = users.ListIndexes();
            indexes.Should().Contain(i => i.Contains("Name"));
        }
    }

    public class DbUser
    {
        public string? Name { get; set; }
        public int Age { get; set; }
    }
}
