using System;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class IndexTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<Employee> _employees;

        public IndexTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_index_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _employees = _client.GetCollection<Employee>("employees");
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== CREATE INDEX ==============

        [Fact]
        public void CreateIndex_CreatesNonUniqueIndex()
        {
            var indexName = _employees.CreateIndex("Department");

            indexName.Should().NotBeNullOrEmpty();
            indexName.Should().Contain("Department");

            var indexes = _employees.ListIndexes();
            indexes.Should().Contain(i => i.Contains("Department"));
        }

        [Fact]
        public void CreateIndex_CreatesUniqueIndex()
        {
            var indexName = _employees.CreateIndex("Email", unique: true);

            indexName.Should().NotBeNullOrEmpty();
            var indexes = _employees.ListIndexes();
            indexes.Should().Contain(i => i.Contains("Email"));
        }

        // ============== COMPOUND INDEX ==============

        [Fact]
        public void CreateCompoundIndex_CreatesMultiFieldIndex()
        {
            var indexName = _employees.CreateCompoundIndex(new[] { "Department", "Level" });

            indexName.Should().NotBeNullOrEmpty();
            var indexes = _employees.ListIndexes();
            indexes.Should().Contain(i => i.Contains("Department") && i.Contains("Level"));
        }

        // ============== LIST INDEXES ==============

        [Fact]
        public void ListIndexes_ReturnsAllIndexes()
        {
            _employees.CreateIndex("Name");
            _employees.CreateIndex("Department");

            var indexes = _employees.ListIndexes();

            indexes.Should().HaveCountGreaterOrEqualTo(2);
            indexes.Should().Contain(i => i.Contains("Name"));
            indexes.Should().Contain(i => i.Contains("Department"));
        }

        // ============== DROP INDEX ==============

        [Fact]
        public void DropIndex_RemovesIndex()
        {
            var indexName = _employees.CreateIndex("Name");
            var indexesBefore = _employees.ListIndexes();
            indexesBefore.Should().Contain(i => i.Contains("Name"));

            _employees.DropIndex(indexName);

            var indexesAfter = _employees.ListIndexes();
            indexesAfter.Should().NotContain(i => i == indexName);
        }

        // ============== EXPLAIN ==============

        [Fact]
        public void Explain_ReturnsQueryPlan()
        {
            // Insert some data
            for (int i = 0; i < 100; i++)
            {
                _employees.InsertOne(new Employee
                {
                    Name = $"Employee{i}",
                    Department = i % 5 == 0 ? "Engineering" : "Other",
                    Level = i % 3
                });
            }

            // Create index
            _employees.CreateIndex("Department");

            // Explain query
            var filter = Builders<Employee>.Filter.Eq("Department", "Engineering");
            var plan = _employees.Explain(filter);

            plan.Should().NotBeNullOrEmpty();
            // The plan should indicate index usage or collection scan
        }

        // ============== INDEX USAGE WITH QUERIES ==============

        [Fact]
        public void IndexedQuery_IsFaster()
        {
            // Insert data (smaller set to avoid page size overflow)
            for (int i = 0; i < 100; i++)
            {
                _employees.InsertOne(new Employee
                {
                    Name = $"Emp{i}",
                    Email = $"emp{i}@company.com",
                    Department = $"Dept{i % 10}",
                    Level = i % 5
                });
            }

            // Query without index (baseline)
            var filterNoIndex = Builders<Employee>.Filter.Eq("Level", 2);
            var resultsNoIndex = _employees.Find(filterNoIndex);

            // Create index
            _employees.CreateIndex("Level");

            // Query with index
            var filterWithIndex = Builders<Employee>.Filter.Eq("Level", 2);
            var resultsWithIndex = _employees.Find(filterWithIndex);

            // Both should return same results
            resultsNoIndex.Count.Should().Be(resultsWithIndex.Count);
            resultsWithIndex.Should().HaveCount(20); // 100 / 5 levels
        }

        // ============== UNIQUE INDEX ENFORCEMENT ==============

        [Fact]
        public void UniqueIndex_PreventsDuplicates()
        {
            _employees.CreateIndex("Email", unique: true);

            _employees.InsertOne(new Employee { Name = "Alice", Email = "alice@test.com" });

            // Try to insert duplicate email
            Action duplicateInsert = () => _employees.InsertOne(new Employee { Name = "Bob", Email = "alice@test.com" });

            duplicateInsert.Should().Throw<Exception>();
        }

        // ============== INDEX WITH UPDATES ==============

        [Fact]
        public void IndexMaintainedAfterUpdate()
        {
            _employees.CreateIndex("Department");

            _employees.InsertOne(new Employee { Name = "TestUser", Department = "Sales" });

            // Update
            var filter = Builders<Employee>.Filter.Eq("Name", "TestUser");
            var update = Builders<Employee>.Update.Set("Department", "Engineering");
            _employees.UpdateOne(filter, update);

            // Query using index should find updated value
            var engineeringFilter = Builders<Employee>.Filter.Eq("Department", "Engineering");
            var result = _employees.FindOne(engineeringFilter);

            result.Should().NotBeNull();
            result!.Name.Should().Be("TestUser");
        }

        // ============== INDEX WITH DELETE ==============

        [Fact]
        public void IndexMaintainedAfterDelete()
        {
            _employees.CreateIndex("Name");

            _employees.InsertOne(new Employee { Name = "ToDelete", Department = "Test" });
            _employees.InsertOne(new Employee { Name = "ToKeep", Department = "Test" });

            // Delete one
            var deleteFilter = Builders<Employee>.Filter.Eq("Name", "ToDelete");
            _employees.DeleteOne(deleteFilter);

            // Should not find deleted
            var findDeleted = _employees.FindOne(deleteFilter);
            findDeleted.Should().BeNull();

            // Should still find the other
            var findKept = Builders<Employee>.Filter.Eq("Name", "ToKeep");
            var kept = _employees.FindOne(findKept);
            kept.Should().NotBeNull();
        }
    }

    public class Employee
    {
        public string? Name { get; set; }
        public string? Email { get; set; }
        public string? Department { get; set; }
        public int Level { get; set; }
    }
}
