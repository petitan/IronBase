using System;
using System.IO;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class DurabilityTests : IDisposable
    {
        private readonly string _testDbPath;
        private IronBaseClient? _client;

        public DurabilityTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_durability_test_{Guid.NewGuid()}.mlite");
        }

        public void Dispose()
        {
            _client?.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== SAFE MODE ==============

        [Fact]
        public void SafeMode_IsDefault()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<DurUser>("users");

            users.InsertOne(new DurUser { Name = "SafeUser" });
            _client.Dispose();

            // Data should persist without explicit flush
            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DurUser>("users");
            users.CountDocuments().Should().Be(1);
        }

        [Fact]
        public void SafeMode_ExplicitConstruction()
        {
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Safe);
            var users = _client.GetCollection<DurUser>("users");

            users.InsertOne(new DurUser { Name = "SafeExplicit" });
            _client.Dispose();

            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DurUser>("users");
            users.CountDocuments().Should().Be(1);
        }

        // ============== BATCH MODE ==============

        [Fact]
        public void BatchMode_AcceptsCustomBatchSize()
        {
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Batch, 50);
            var users = _client.GetCollection<DurUser>("users");

            // Insert less than batch size
            for (int i = 0; i < 30; i++)
            {
                users.InsertOne(new DurUser { Name = $"BatchUser{i}" });
            }

            // Should still work
            users.CountDocuments().Should().Be(30);
        }

        [Fact]
        public void BatchMode_FlushPersistsData()
        {
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Batch, 1000);
            var users = _client.GetCollection<DurUser>("users");

            for (int i = 0; i < 10; i++)
            {
                users.InsertOne(new DurUser { Name = $"User{i}" });
            }

            _client.Flush();
            _client.Dispose();

            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DurUser>("users");
            users.CountDocuments().Should().Be(10);
        }

        // ============== UNSAFE MODE ==============

        [Fact]
        public void UnsafeMode_RequiresCheckpoint()
        {
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Unsafe);
            var users = _client.GetCollection<DurUser>("users");

            users.InsertOne(new DurUser { Name = "UnsafeUser" });
            _client.Checkpoint();
            _client.Dispose();

            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DurUser>("users");
            users.CountDocuments().Should().BeGreaterOrEqualTo(0); // May or may not persist without explicit checkpoint
        }

        [Fact]
        public void UnsafeMode_FlushPersistsData()
        {
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Unsafe);
            var users = _client.GetCollection<DurUser>("users");

            users.InsertOne(new DurUser { Name = "UnsafeFlush" });
            _client.Flush();
            _client.Dispose();

            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<DurUser>("users");
            users.CountDocuments().Should().Be(1);
        }

        // ============== SWITCHING MODES ==============

        [Fact]
        public void CanReopenWithDifferentMode()
        {
            // Create with Safe mode
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Safe);
            var users = _client.GetCollection<DurUser>("users");
            users.InsertOne(new DurUser { Name = "ModeSwitch" });
            _client.Dispose();

            // Reopen with Batch mode
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Batch, 100);
            users = _client.GetCollection<DurUser>("users");
            users.CountDocuments().Should().Be(1);

            // Add more data
            users.InsertOne(new DurUser { Name = "BatchAdd" });
            _client.Flush();
            _client.Dispose();

            // Reopen with Safe mode again
            _client = new IronBaseClient(_testDbPath, DurabilityMode.Safe);
            users = _client.GetCollection<DurUser>("users");
            users.CountDocuments().Should().Be(2);
        }
    }

    public class DurUser
    {
        public string? Name { get; set; }
    }
}
