using System;
using System.IO;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class TransactionTests : IDisposable
    {
        private readonly string _testDbPath;
        private IronBaseClient? _client;

        public TransactionTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_tx_test_{Guid.NewGuid()}.mlite");
        }

        public void Dispose()
        {
            _client?.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== BEGIN TRANSACTION ==============

        [Fact]
        public void BeginTransaction_ReturnsValidId()
        {
            _client = new IronBaseClient(_testDbPath);

            var txId = _client.BeginTransaction();

            txId.Should().BeGreaterThan(0);
        }

        [Fact]
        public void BeginTransaction_EachTransactionGetsUniqueId()
        {
            _client = new IronBaseClient(_testDbPath);

            var txId1 = _client.BeginTransaction();
            _client.CommitTransaction(txId1);

            var txId2 = _client.BeginTransaction();
            _client.CommitTransaction(txId2);

            txId2.Should().NotBe(txId1);
        }

        // ============== COMMIT TRANSACTION ==============

        [Fact]
        public void CommitTransaction_CommitsChanges()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TxUser>("users");

            var txId = _client.BeginTransaction();
            users.InsertOne(new TxUser { Name = "TxUser", Age = 30 });
            _client.CommitTransaction(txId);

            // Changes should be visible after commit
            var count = users.CountDocuments();
            count.Should().Be(1);
        }

        [Fact]
        public void CommitTransaction_PersistsAfterReopen()
        {
            // Insert in transaction
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TxUser>("users");

            var txId = _client.BeginTransaction();
            users.InsertOne(new TxUser { Name = "Persistent", Age = 25 });
            _client.CommitTransaction(txId);
            _client.Dispose();

            // Reopen and verify
            _client = new IronBaseClient(_testDbPath);
            users = _client.GetCollection<TxUser>("users");

            var found = users.FindOne(Builders<TxUser>.Filter.Eq("Name", "Persistent"));
            found.Should().NotBeNull();
            found!.Name.Should().Be("Persistent");
        }

        // ============== ROLLBACK TRANSACTION ==============

        [Fact]
        public void RollbackTransaction_DoesNotThrow()
        {
            // Note: IronBase's rollback behavior depends on the durability mode
            // In Safe mode (default), operations are committed immediately,
            // so rollback may not revert already-committed changes.
            // This test verifies that rollback doesn't throw an exception.
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TxUser>("users");

            var txId = _client.BeginTransaction();
            users.InsertOne(new TxUser { Name = "TestUser", Age = 30 });

            // Rollback should not throw
            Action rollback = () => _client.RollbackTransaction(txId);
            rollback.Should().NotThrow();
        }

        // ============== MULTIPLE OPERATIONS IN TRANSACTION ==============

        [Fact]
        public void Transaction_MultipleInserts()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TxUser>("users");

            var txId = _client.BeginTransaction();
            users.InsertOne(new TxUser { Name = "User1", Age = 20 });
            users.InsertOne(new TxUser { Name = "User2", Age = 25 });
            users.InsertOne(new TxUser { Name = "User3", Age = 30 });
            _client.CommitTransaction(txId);

            users.CountDocuments().Should().Be(3);
        }

        [Fact]
        public void Transaction_MixedOperations()
        {
            _client = new IronBaseClient(_testDbPath);
            var users = _client.GetCollection<TxUser>("users");

            // Setup
            users.InsertOne(new TxUser { Name = "Existing", Age = 40 });

            // Transaction with mixed operations
            var txId = _client.BeginTransaction();
            users.InsertOne(new TxUser { Name = "NewUser", Age = 25 });
            users.UpdateOne(
                Builders<TxUser>.Filter.Eq("Name", "Existing"),
                Builders<TxUser>.Update.Set("Age", 41)
            );
            _client.CommitTransaction(txId);

            // Verify both operations
            users.CountDocuments().Should().Be(2);
            var updated = users.FindOne(Builders<TxUser>.Filter.Eq("Name", "Existing"));
            updated!.Age.Should().Be(41);
        }
    }

    public class TxUser
    {
        public string? Name { get; set; }
        public int Age { get; set; }
    }
}
