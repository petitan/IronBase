using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    /// <summary>
    /// Tests for recursive wildcard ($**) operator and large-scale data operations.
    /// </summary>
    public class WildcardAndScaleTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;

        public WildcardAndScaleTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_wildcard_test_{Guid.NewGuid()}.mlite");
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

        // ============== RECURSIVE WILDCARD ($**) TESTS ==============

        [Fact]
        public void RecursiveWildcard_FindsFieldAtTopLevel()
        {
            var collection = _client.GetCollection<NestedDocument>("wildcard_test");

            collection.InsertOne(new NestedDocument
            {
                Name = "Test1",
                Status = "active"
            });
            collection.InsertOne(new NestedDocument
            {
                Name = "Test2",
                Status = "inactive"
            });

            // $**.Status should find the top-level Status field
            var filter = Builders<NestedDocument>.Filter.RecursiveWildcard("Status", "active");
            var results = collection.Find(filter);

            results.Should().HaveCount(1);
            results.First().Name.Should().Be("Test1");
        }

        [Fact]
        public void RecursiveWildcard_FindsFieldInNestedObject()
        {
            var collection = _client.GetCollection<DocumentWithNested>("nested_wildcard");

            collection.InsertOne(new DocumentWithNested
            {
                Id = "1",
                Metadata = new NestedInfo { Status = "active", Priority = 1 }
            });
            collection.InsertOne(new DocumentWithNested
            {
                Id = "2",
                Metadata = new NestedInfo { Status = "inactive", Priority = 2 }
            });

            // $**.Status should find Status inside Metadata
            var filter = Builders<DocumentWithNested>.Filter.RecursiveWildcard("Status", "active");
            var results = collection.Find(filter);

            results.Should().HaveCount(1);
            results.First().Id.Should().Be("1");
        }

        [Fact]
        public void RecursiveWildcard_FindsFieldAtAnyDepth()
        {
            var collection = _client.GetCollection<BsonDocument>("deep_wildcard");

            // Use BsonDocument for dynamic nested structure
            var doc1 = new BsonDocument
            {
                ["Id"] = "1",
                ["Level1"] = new BsonDocument
                {
                    ["Level2"] = new BsonDocument
                    {
                        ["Level3"] = new BsonDocument
                        {
                            ["Target"] = "found"
                        }
                    }
                }
            };
            var doc2 = new BsonDocument
            {
                ["Id"] = "2",
                ["Level1"] = new BsonDocument
                {
                    ["Level2"] = new BsonDocument
                    {
                        ["Level3"] = new BsonDocument
                        {
                            ["Target"] = "not_this"
                        }
                    }
                }
            };
            var doc3 = new BsonDocument
            {
                ["Id"] = "3",
                ["Target"] = "at_root"
            };

            collection.InsertOne(doc1);
            collection.InsertOne(doc2);
            collection.InsertOne(doc3);

            // $**.Target should find at any depth
            var filter = Builders<BsonDocument>.Filter.RecursiveWildcard("Target", "found");
            var results = collection.Find(filter);

            results.Should().HaveCount(1);
        }

        [Fact]
        public void RecursiveWildcard_WithComparison_Gt()
        {
            var collection = _client.GetCollection<DocumentWithNested>("wildcard_gt");

            collection.InsertOne(new DocumentWithNested
            {
                Id = "1",
                Metadata = new NestedInfo { Status = "active", Priority = 10 }
            });
            collection.InsertOne(new DocumentWithNested
            {
                Id = "2",
                Metadata = new NestedInfo { Status = "active", Priority = 5 }
            });

            // $**.Priority with $gt
            var filter = Builders<DocumentWithNested>.Filter.RecursiveWildcard("Priority", "$gt", 7);
            var results = collection.Find(filter);

            results.Should().HaveCount(1);
            results.First().Id.Should().Be("1");
        }

        [Fact]
        public void RecursiveWildcard_WithRegex()
        {
            var collection = _client.GetCollection<BsonDocument>("wildcard_regex");

            collection.InsertOne(new BsonDocument
            {
                ["Id"] = "1",
                ["Data"] = new BsonDocument
                {
                    ["Content"] = "This contains important info"
                }
            });
            collection.InsertOne(new BsonDocument
            {
                ["Id"] = "2",
                ["Data"] = new BsonDocument
                {
                    ["Content"] = "Nothing special"
                }
            });

            // $**.Content with $regex
            var filter = Builders<BsonDocument>.Filter.RecursiveWildcard("Content", "$regex", "important");
            var results = collection.Find(filter);

            results.Should().HaveCount(1);
        }

        [Fact]
        public void RecursiveWildcard_WithJsonFilter()
        {
            var collection = _client.GetCollection<BsonDocument>("wildcard_json");

            collection.InsertOne(new BsonDocument
            {
                ["Id"] = "doc1",
                ["Nested"] = new BsonDocument
                {
                    ["Deep"] = new BsonDocument
                    {
                        ["Value"] = 42
                    }
                }
            });
            collection.InsertOne(new BsonDocument
            {
                ["Id"] = "doc2",
                ["Nested"] = new BsonDocument
                {
                    ["Deep"] = new BsonDocument
                    {
                        ["Value"] = 100
                    }
                }
            });

            // Direct JSON filter with $**
            var results = collection.Find("{\"$**.Value\": 42}");

            results.Should().HaveCount(1);
        }

        [Fact]
        public void RecursiveWildcard_InArray()
        {
            var collection = _client.GetCollection<BsonDocument>("wildcard_array");

            collection.InsertOne(new BsonDocument
            {
                ["Id"] = "1",
                ["Items"] = new List<object>
                {
                    new BsonDocument { ["Name"] = "Apple" },
                    new BsonDocument { ["Name"] = "Banana" }
                }
            });
            collection.InsertOne(new BsonDocument
            {
                ["Id"] = "2",
                ["Items"] = new List<object>
                {
                    new BsonDocument { ["Name"] = "Cherry" },
                    new BsonDocument { ["Name"] = "Date" }
                }
            });

            // $**.Name should search inside array elements
            var results = collection.Find("{\"$**.Name\": \"Apple\"}");

            results.Should().HaveCount(1);
        }

        // ============== LARGE DATA TESTS ==============

        [Fact]
        public void LargeData_Insert10000Documents()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_insert");
            var docs = new List<SimpleDoc>();

            for (int i = 0; i < 10000; i++)
            {
                docs.Add(new SimpleDoc { Id = i, Name = $"Doc_{i}", Value = i * 10 });
            }

            var sw = Stopwatch.StartNew();
            collection.InsertMany(docs);
            sw.Stop();

            var count = collection.CountDocuments();
            count.Should().Be(10000);

            // Insertion should complete in reasonable time (< 30 seconds for 10k docs)
            sw.ElapsedMilliseconds.Should().BeLessThan(30000);
        }

        [Fact]
        public void LargeData_QueryPerformance()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_query");
            var docs = new List<SimpleDoc>();

            for (int i = 0; i < 5000; i++)
            {
                docs.Add(new SimpleDoc { Id = i, Name = $"Doc_{i % 100}", Value = i });
            }
            collection.InsertMany(docs);

            var sw = Stopwatch.StartNew();
            var filter = Builders<SimpleDoc>.Filter.Eq("Name", "Doc_42");
            var results = collection.Find(filter);
            sw.Stop();

            results.Should().HaveCount(50); // 5000 / 100 = 50
            sw.ElapsedMilliseconds.Should().BeLessThan(5000);
        }

        [Fact]
        public void LargeData_AggregationOnManyDocs()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_agg");
            var docs = new List<SimpleDoc>();

            for (int i = 0; i < 5000; i++)
            {
                docs.Add(new SimpleDoc
                {
                    Id = i,
                    Name = $"Category_{i % 10}",
                    Value = i
                });
            }
            collection.InsertMany(docs);

            var sw = Stopwatch.StartNew();
            var pipeline = "[{\"$group\": {\"_id\": \"$Name\", \"total\": {\"$sum\": \"$Value\"}}}]";
            var results = collection.Aggregate<BsonDocument>(pipeline);
            sw.Stop();

            results.Should().HaveCount(10); // 10 unique categories
            sw.ElapsedMilliseconds.Should().BeLessThan(10000);
        }

        [Fact]
        public void LargeData_WithIndex()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_index");

            // Create index BEFORE inserting docs to avoid node size limit
            collection.CreateIndex("Id");

            var docs = new List<SimpleDoc>();
            for (int i = 0; i < 1000; i++)
            {
                docs.Add(new SimpleDoc { Id = i, Name = $"Name_{i}", Value = i });
            }
            collection.InsertMany(docs);

            var sw = Stopwatch.StartNew();
            var filter = Builders<SimpleDoc>.Filter.Eq("Id", 500);
            var results = collection.Find(filter);
            sw.Stop();

            results.Should().HaveCount(1);
            results.First().Id.Should().Be(500);
            // With index, query should be fast
            sw.ElapsedMilliseconds.Should().BeLessThan(1000);
        }

        [Fact]
        public void LargeData_BulkUpdatePerformance()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_update");
            var docs = new List<SimpleDoc>();

            for (int i = 0; i < 3000; i++)
            {
                docs.Add(new SimpleDoc { Id = i, Name = "ToUpdate", Value = i });
            }
            collection.InsertMany(docs);

            var sw = Stopwatch.StartNew();
            var filter = Builders<SimpleDoc>.Filter.Eq("Name", "ToUpdate");
            var update = Builders<SimpleDoc>.Update.Set("Name", "Updated");
            var result = collection.UpdateMany(filter, update);
            sw.Stop();

            result.ModifiedCount.Should().Be(3000);
            sw.ElapsedMilliseconds.Should().BeLessThan(30000);
        }

        [Fact]
        public void LargeData_BulkDeletePerformance()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_delete");
            var docs = new List<SimpleDoc>();

            for (int i = 0; i < 2000; i++)
            {
                docs.Add(new SimpleDoc { Id = i, Name = i % 2 == 0 ? "ToDelete" : "ToKeep", Value = i });
            }
            collection.InsertMany(docs);

            var sw = Stopwatch.StartNew();
            var filter = Builders<SimpleDoc>.Filter.Eq("Name", "ToDelete");
            var result = collection.DeleteMany(filter);
            sw.Stop();

            result.DeletedCount.Should().Be(1000);
            sw.ElapsedMilliseconds.Should().BeLessThan(30000);

            var remaining = collection.CountDocuments();
            remaining.Should().Be(1000);
        }

        [Fact]
        public void LargeData_DistinctOnManyDocs()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_distinct");
            var docs = new List<SimpleDoc>();

            for (int i = 0; i < 5000; i++)
            {
                docs.Add(new SimpleDoc
                {
                    Id = i,
                    Name = $"Name_{i % 100}", // 100 unique names
                    Value = i
                });
            }
            collection.InsertMany(docs);

            var sw = Stopwatch.StartNew();
            var distinctNames = collection.Distinct<string>("Name");
            sw.Stop();

            distinctNames.Should().HaveCount(100);
            sw.ElapsedMilliseconds.Should().BeLessThan(5000);
        }

        [Fact]
        public void LargeData_PaginationPerformance()
        {
            var collection = _client.GetCollection<SimpleDoc>("large_pagination");
            var docs = new List<SimpleDoc>();

            for (int i = 0; i < 5000; i++)
            {
                docs.Add(new SimpleDoc { Id = i, Name = $"Doc_{i}", Value = i });
            }
            collection.InsertMany(docs);

            var options = new FindOptions
            {
                Skip = 4000,
                Limit = 100,
                Sort = new List<(string Field, int Direction)> { ("Id", 1) }
            };

            var sw = Stopwatch.StartNew();
            var filter = Builders<SimpleDoc>.Filter.Empty;
            var results = collection.Find(filter, options);
            sw.Stop();

            results.Should().HaveCount(100);
            // First result after skip should be Id 4000
            results.First().Id.Should().BeGreaterOrEqualTo(4000);
            sw.ElapsedMilliseconds.Should().BeLessThan(5000);
        }
    }

    // Test model classes
    public class NestedDocument
    {
        public string? Name { get; set; }
        public string? Status { get; set; }
    }

    public class NestedInfo
    {
        public string? Status { get; set; }
        public int Priority { get; set; }
    }

    public class DocumentWithNested
    {
        public string? Id { get; set; }
        public NestedInfo? Metadata { get; set; }
    }

    public class SimpleDoc
    {
        public int Id { get; set; }
        public string? Name { get; set; }
        public int Value { get; set; }
    }
}
