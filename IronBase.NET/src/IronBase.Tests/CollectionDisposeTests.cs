using System;
using System.IO;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    /// <summary>
    /// #41: a collection releases its native handle on Dispose (or when the
    /// garbage collector finalizes it), and refuses use after it or after its
    /// client is disposed.
    /// </summary>
    public class CollectionDisposeTests : IDisposable
    {
        public class Item
        {
            public string Name { get; set; } = "";
        }

        private readonly string _testDbPath;
        private readonly IronBaseClient _client;

        public CollectionDisposeTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        [Fact]
        public void DisposedCollection_ThrowsObjectDisposed()
        {
            var users = _client.GetCollection<Item>("users");
            users.InsertOne(new Item { Name = "Alice" });

            users.Dispose();
            users.Dispose(); // a second Dispose is a no-op

            Action use = () => users.CountDocuments();
            use.Should().Throw<ObjectDisposedException>();

            // The data stays; another collection object reads it
            using var again = _client.GetCollection<Item>("users");
            again.CountDocuments().Should().Be(1);
        }

        [Fact]
        public void CollectionOfDisposedClient_ThrowsObjectDisposed()
        {
            var path = Path.Combine(Path.GetTempPath(), $"ironbase_test_{Guid.NewGuid()}.mlite");
            var client = new IronBaseClient(path);
            var users = client.GetCollection<Item>("users");
            client.Dispose();
            try
            {
                Action use = () => users.CountDocuments();
                use.Should().Throw<ObjectDisposedException>();
                users.Dispose();
            }
            finally
            {
                File.Delete(path);
                if (File.Exists(path + ".wal"))
                    File.Delete(path + ".wal");
            }
        }

        [Fact]
        public void UndisposedCollections_AreReleasedByTheFinalizer()
        {
            // Every GetCollection used to keep a native handle (and the whole
            // database) alive until exit; now the GC releases them
            for (int i = 0; i < 1000; i++)
            {
                _client.GetCollection<Item>("users").CountDocuments();
            }
            GC.Collect();
            GC.WaitForPendingFinalizers();
            _client.GetCollection<Item>("users").CountDocuments().Should().Be(0);
        }
    }
}
