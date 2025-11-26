using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class BulkOperationTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<Log> _logs;

        public BulkOperationTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_bulk_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _logs = _client.GetCollection<Log>("logs");
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== INSERT MANY ==============

        [Fact]
        public void InsertMany_InsertsMultipleDocuments()
        {
            var logs = new List<Log>
            {
                new Log { Level = "INFO", Message = "Start" },
                new Log { Level = "DEBUG", Message = "Processing" },
                new Log { Level = "INFO", Message = "Complete" }
            };

            var result = _logs.InsertMany(logs);

            result.Acknowledged.Should().BeTrue();
            result.InsertedCount.Should().Be(3);

            _logs.CountDocuments().Should().Be(3);
        }

        [Fact]
        public void InsertMany_EmptyList()
        {
            var logs = new List<Log>();
            var result = _logs.InsertMany(logs);

            result.Acknowledged.Should().BeTrue();
            result.InsertedCount.Should().Be(0);
        }

        [Fact]
        public void InsertMany_LargeBatch()
        {
            var logs = Enumerable.Range(0, 500).Select(i => new Log
            {
                Level = i % 3 == 0 ? "ERROR" : "INFO",
                Message = $"Message {i}",
                Code = i
            }).ToList();

            var result = _logs.InsertMany(logs);

            result.Acknowledged.Should().BeTrue();
            result.InsertedCount.Should().Be(500);

            _logs.CountDocuments().Should().Be(500);
        }

        // ============== DELETE MANY ==============

        [Fact]
        public void DeleteMany_DeletesAllMatching()
        {
            // Insert test data
            var logs = new List<Log>
            {
                new Log { Level = "ERROR", Message = "Error 1" },
                new Log { Level = "ERROR", Message = "Error 2" },
                new Log { Level = "INFO", Message = "Info 1" },
                new Log { Level = "ERROR", Message = "Error 3" }
            };
            _logs.InsertMany(logs);

            // Delete all errors
            var filter = Builders<Log>.Filter.Eq("Level", "ERROR");
            var result = _logs.DeleteMany(filter);

            result.DeletedCount.Should().Be(3);
            _logs.CountDocuments().Should().Be(1);

            var remaining = _logs.FindOne();
            remaining!.Level.Should().Be("INFO");
        }

        [Fact]
        public void DeleteMany_NoMatches()
        {
            _logs.InsertMany(new List<Log>
            {
                new Log { Level = "INFO", Message = "Test" }
            });

            var filter = Builders<Log>.Filter.Eq("Level", "CRITICAL");
            var result = _logs.DeleteMany(filter);

            result.DeletedCount.Should().Be(0);
            _logs.CountDocuments().Should().Be(1);
        }

        [Fact]
        public void DeleteMany_WithComplexFilter()
        {
            // Insert test data
            var logs = Enumerable.Range(0, 100).Select(i => new Log
            {
                Level = i % 2 == 0 ? "ERROR" : "INFO",
                Message = $"Message {i}",
                Code = i
            }).ToList();
            _logs.InsertMany(logs);

            // Delete errors with code > 50
            var filter = Builders<Log>.Filter.And(
                Builders<Log>.Filter.Eq("Level", "ERROR"),
                Builders<Log>.Filter.Gt("Code", 50)
            );
            var result = _logs.DeleteMany(filter);

            // Even numbers > 50: 52, 54, 56, ..., 98 = 24 numbers
            result.DeletedCount.Should().Be(24);
        }

        // ============== MIXED OPERATIONS ==============

        [Fact]
        public void BulkInsertThenUpdateMany()
        {
            // Insert
            var logs = Enumerable.Range(0, 50).Select(i => new Log
            {
                Level = "PENDING",
                Message = $"Task {i}",
                Code = i
            }).ToList();
            _logs.InsertMany(logs);

            // Update all to PROCESSED
            var filter = Builders<Log>.Filter.Eq("Level", "PENDING");
            var update = Builders<Log>.Update.Set("Level", "PROCESSED");
            var result = _logs.UpdateMany(filter, update);

            result.MatchedCount.Should().Be(50);
            result.ModifiedCount.Should().Be(50);

            var processedCount = _logs.CountDocuments(Builders<Log>.Filter.Eq("Level", "PROCESSED"));
            processedCount.Should().Be(50);
        }

        [Fact]
        public void BulkInsertQueryDeleteSequence()
        {
            // Insert
            var logs = Enumerable.Range(0, 100).Select(i => new Log
            {
                Level = i < 30 ? "OLD" : "NEW",
                Message = $"Log {i}",
                Code = i
            }).ToList();
            _logs.InsertMany(logs);
            _logs.CountDocuments().Should().Be(100);

            // Query
            var oldLogs = _logs.Find(Builders<Log>.Filter.Eq("Level", "OLD"));
            oldLogs.Should().HaveCount(30);

            // Delete old
            var deleteResult = _logs.DeleteMany(Builders<Log>.Filter.Eq("Level", "OLD"));
            deleteResult.DeletedCount.Should().Be(30);

            // Verify
            _logs.CountDocuments().Should().Be(70);
            var remainingOld = _logs.CountDocuments(Builders<Log>.Filter.Eq("Level", "OLD"));
            remainingOld.Should().Be(0);
        }

        // ============== PERFORMANCE ==============

        [Fact]
        public void InsertMany_PerformsWell()
        {
            var logs = Enumerable.Range(0, 1000).Select(i => new Log
            {
                Level = "INFO",
                Message = $"Performance test log {i}",
                Code = i
            }).ToList();

            var sw = System.Diagnostics.Stopwatch.StartNew();
            var result = _logs.InsertMany(logs);
            sw.Stop();

            result.InsertedCount.Should().Be(1000);
            // Should complete in reasonable time (< 5 seconds)
            sw.ElapsedMilliseconds.Should().BeLessThan(5000);
        }
    }

    public class Log
    {
        public string? Level { get; set; }
        public string? Message { get; set; }
        public int Code { get; set; }
    }
}
