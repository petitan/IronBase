using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using FluentAssertions;
using Xunit;

namespace IronBase.Tests
{
    public class NestedDocumentTests : IDisposable
    {
        private readonly string _testDbPath;
        private readonly IronBaseClient _client;
        private readonly IronBaseCollection<Person> _people;

        public NestedDocumentTests()
        {
            _testDbPath = Path.Combine(Path.GetTempPath(), $"ironbase_nested_test_{Guid.NewGuid()}.mlite");
            _client = new IronBaseClient(_testDbPath);
            _people = _client.GetCollection<Person>("people");
        }

        public void Dispose()
        {
            _client.Dispose();
            if (File.Exists(_testDbPath))
                File.Delete(_testDbPath);
            if (File.Exists(_testDbPath + ".wal"))
                File.Delete(_testDbPath + ".wal");
        }

        // ============== NESTED OBJECTS ==============

        [Fact]
        public void InsertAndRetrieve_NestedObject()
        {
            var person = new Person
            {
                Name = "Alice",
                Address = new Address
                {
                    Street = "123 Main St",
                    City = "Springfield",
                    ZipCode = "12345"
                }
            };

            _people.InsertOne(person);

            var found = _people.FindOne(Builders<Person>.Filter.Eq("Name", "Alice"));

            found.Should().NotBeNull();
            found!.Address.Should().NotBeNull();
            found.Address!.Street.Should().Be("123 Main St");
            found.Address.City.Should().Be("Springfield");
        }

        [Fact]
        public void Query_NestedField()
        {
            _people.InsertOne(new Person
            {
                Name = "Bob",
                Address = new Address { City = "New York" }
            });
            _people.InsertOne(new Person
            {
                Name = "Charlie",
                Address = new Address { City = "Los Angeles" }
            });

            // Query by nested field using dot notation
            var filter = Builders<Person>.Filter.Eq("Address.City", "New York");
            var results = _people.Find(filter);

            results.Should().HaveCount(1);
            results[0].Name.Should().Be("Bob");
        }

        // ============== ARRAYS ==============

        [Fact]
        public void InsertAndRetrieve_ArrayField()
        {
            var person = new Person
            {
                Name = "Diana",
                Tags = new List<string> { "developer", "manager", "speaker" }
            };

            _people.InsertOne(person);

            var found = _people.FindOne(Builders<Person>.Filter.Eq("Name", "Diana"));

            found.Should().NotBeNull();
            found!.Tags.Should().HaveCount(3);
            found.Tags.Should().Contain("developer", "manager", "speaker");
        }

        [Fact]
        public void Query_ArrayField_RetrievesCorrectly()
        {
            _people.InsertOne(new Person { Name = "Eve", Tags = new List<string> { "admin", "user" } });
            _people.InsertOne(new Person { Name = "Frank", Tags = new List<string> { "user", "guest" } });

            var results = _people.Find();

            results.Should().HaveCount(2);
            var eve = results.First(p => p.Name == "Eve");
            eve.Tags.Should().Contain("admin", "user");
        }

        [Fact]
        public void Query_ArrayElementMatching_Eq()
        {
            // MongoDB-style array element matching: {Tags: "admin"} matches if array contains "admin"
            _people.InsertOne(new Person { Name = "Eve", Tags = new List<string> { "admin", "user" } });
            _people.InsertOne(new Person { Name = "Frank", Tags = new List<string> { "user", "guest" } });

            // Query for documents where Tags array contains "admin"
            var filter = Builders<Person>.Filter.Eq("Tags", "admin");
            var results = _people.Find(filter);

            results.Should().HaveCount(1);
            results[0].Name.Should().Be("Eve");
        }

        [Fact]
        public void Query_ArrayElementMatching_In()
        {
            // MongoDB-style $in with array: matches if any element in doc array matches any value in filter array
            _people.InsertOne(new Person { Name = "Eve", Tags = new List<string> { "admin", "user" } });
            _people.InsertOne(new Person { Name = "Frank", Tags = new List<string> { "user", "guest" } });
            _people.InsertOne(new Person { Name = "Grace", Tags = new List<string> { "developer" } });

            // Query for documents where Tags array has any element in ["admin", "guest"]
            var filter = Builders<Person>.Filter.In("Tags", new[] { "admin", "guest" });
            var results = _people.Find(filter);

            results.Should().HaveCount(2);
            results.Select(p => p.Name).Should().Contain("Eve", "Frank");
        }

        // ============== ARRAY OF OBJECTS ==============

        [Fact]
        public void InsertAndRetrieve_ArrayOfObjects()
        {
            var person = new Person
            {
                Name = "Henry",
                Contacts = new List<Contact>
                {
                    new Contact { Type = "email", Value = "henry@example.com" },
                    new Contact { Type = "phone", Value = "555-1234" }
                }
            };

            _people.InsertOne(person);

            var found = _people.FindOne(Builders<Person>.Filter.Eq("Name", "Henry"));

            found.Should().NotBeNull();
            found!.Contacts.Should().HaveCount(2);
            found.Contacts![0].Type.Should().Be("email");
            found.Contacts[1].Value.Should().Be("555-1234");
        }

        // ============== DEEPLY NESTED ==============

        [Fact]
        public void InsertAndRetrieve_DeeplyNested()
        {
            var person = new Person
            {
                Name = "Iris",
                Address = new Address
                {
                    Street = "456 Oak Ave",
                    City = "Boston",
                    Coordinates = new GeoCoordinates
                    {
                        Latitude = 42.3601,
                        Longitude = -71.0589
                    }
                }
            };

            _people.InsertOne(person);

            var found = _people.FindOne(Builders<Person>.Filter.Eq("Name", "Iris"));

            found.Should().NotBeNull();
            found!.Address!.Coordinates.Should().NotBeNull();
            found.Address.Coordinates!.Latitude.Should().BeApproximately(42.3601, 0.0001);
            found.Address.Coordinates.Longitude.Should().BeApproximately(-71.0589, 0.0001);
        }

        // ============== UPDATE NESTED FIELDS WITH DOT NOTATION ==============

        [Fact]
        public void Update_NestedField_WithDotNotation()
        {
            // Test that dot notation works for nested field updates (MongoDB-style)
            _people.InsertOne(new Person
            {
                Name = "Jack",
                Age = 30,
                Address = new Address { City = "Seattle", ZipCode = "98101" }
            });

            var filter = Builders<Person>.Filter.Eq("Name", "Jack");

            // Update nested field using dot notation
            var update = Builders<Person>.Update.Set("Address.City", "Portland");
            _people.UpdateOne(filter, update);

            var found = _people.FindOne(filter);
            found!.Address!.City.Should().Be("Portland"); // City updated via dot notation
            found.Address.ZipCode.Should().Be("98101"); // ZipCode preserved
        }

        [Fact]
        public void Update_TopLevelField_PreservesNestedObjects()
        {
            _people.InsertOne(new Person
            {
                Name = "Kate",
                Age = 30,
                Address = new Address { City = "Seattle", ZipCode = "98101" }
            });

            var filter = Builders<Person>.Filter.Eq("Name", "Kate");
            var update = Builders<Person>.Update.Set("Age", 31);
            _people.UpdateOne(filter, update);

            var found = _people.FindOne(filter);
            found!.Age.Should().Be(31);
            found.Address!.City.Should().Be("Seattle"); // Nested object preserved
            found.Address.ZipCode.Should().Be("98101");
        }

        // ============== NULL NESTED OBJECTS ==============

        [Fact]
        public void InsertAndRetrieve_NullNestedObject()
        {
            var person = new Person
            {
                Name = "Kate",
                Address = null
            };

            _people.InsertOne(person);

            var found = _people.FindOne(Builders<Person>.Filter.Eq("Name", "Kate"));

            found.Should().NotBeNull();
            found!.Address.Should().BeNull();
        }

        // ============== EMPTY ARRAYS ==============

        [Fact]
        public void InsertAndRetrieve_EmptyArray()
        {
            var person = new Person
            {
                Name = "Leo",
                Tags = new List<string>()
            };

            _people.InsertOne(person);

            var found = _people.FindOne(Builders<Person>.Filter.Eq("Name", "Leo"));

            found.Should().NotBeNull();
            found!.Tags.Should().BeEmpty();
        }

        // ============== MIXED TYPES ==============

        [Fact]
        public void InsertAndRetrieve_ComplexDocument()
        {
            var person = new Person
            {
                Name = "Mike",
                Age = 35,
                Active = true,
                Score = 95.5,
                Address = new Address
                {
                    Street = "789 Pine Rd",
                    City = "Denver"
                },
                Tags = new List<string> { "premium", "verified" },
                Contacts = new List<Contact>
                {
                    new Contact { Type = "email", Value = "mike@example.com" }
                }
            };

            _people.InsertOne(person);

            var found = _people.FindOne(Builders<Person>.Filter.Eq("Name", "Mike"));

            found.Should().NotBeNull();
            found!.Age.Should().Be(35);
            found.Active.Should().BeTrue();
            found.Score.Should().BeApproximately(95.5, 0.01);
            found.Address!.City.Should().Be("Denver");
            found.Tags.Should().HaveCount(2);
            found.Contacts.Should().HaveCount(1);
        }
    }

    public class Person
    {
        public string? Name { get; set; }
        public int Age { get; set; }
        public bool Active { get; set; }
        public double Score { get; set; }
        public Address? Address { get; set; }
        public List<string>? Tags { get; set; }
        public List<Contact>? Contacts { get; set; }
    }

    public class Address
    {
        public string? Street { get; set; }
        public string? City { get; set; }
        public string? ZipCode { get; set; }
        public GeoCoordinates? Coordinates { get; set; }
    }

    public class GeoCoordinates
    {
        public double Latitude { get; set; }
        public double Longitude { get; set; }
    }

    public class Contact
    {
        public string? Type { get; set; }
        public string? Value { get; set; }
    }
}
