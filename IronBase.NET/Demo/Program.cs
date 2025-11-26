using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;
using IronBase;

// ============================================================
// IronBase C# Demo - Comprehensive Feature Showcase
// ============================================================

Console.WriteLine("╔══════════════════════════════════════════════════════════╗");
Console.WriteLine("║           IronBase C# Demo - Feature Showcase            ║");
Console.WriteLine("╚══════════════════════════════════════════════════════════╝");
Console.WriteLine();

// Clean up old database file
var dbPath = "demo.mlite";
if (File.Exists(dbPath)) File.Delete(dbPath);
if (File.Exists(dbPath + ".wal")) File.Delete(dbPath + ".wal");

// Open database with Safe durability mode (default)
using var client = new IronBaseClient(dbPath);

Console.WriteLine($"IronBase Version: {IronBaseClient.GetVersion()}");
Console.WriteLine($"Database path: {client.Path}");
Console.WriteLine();

// ============================================================
// 1. BASIC CRUD OPERATIONS
// ============================================================
PrintSection("1. BASIC CRUD OPERATIONS");

var users = client.GetCollection<User>("users");

// Insert One
Console.WriteLine(">>> InsertOne");
var insertResult = users.InsertOne(new User
{
    Name = "Alice",
    Age = 30,
    City = "New York",
    Email = "alice@example.com",
    Tags = new[] { "developer", "team-lead" },
    Profile = new UserProfile { Score = 95, Level = "senior" }
});
Console.WriteLine($"    Inserted ID: {insertResult.InsertedId}");

// Insert Many
Console.WriteLine("\n>>> InsertMany");
var manyResult = users.InsertMany(new[]
{
    new User { Name = "Bob", Age = 25, City = "Los Angeles", Email = "bob@example.com",
               Tags = new[] { "developer" }, Profile = new UserProfile { Score = 82, Level = "mid" } },
    new User { Name = "Carol", Age = 35, City = "New York", Email = "carol@example.com",
               Tags = new[] { "manager", "team-lead" }, Profile = new UserProfile { Score = 88, Level = "senior" } },
    new User { Name = "David", Age = 28, City = "Chicago", Email = "david@example.com",
               Tags = new[] { "developer", "intern" }, Profile = new UserProfile { Score = 75, Level = "junior" } },
    new User { Name = "Eve", Age = 32, City = "New York", Email = "eve@example.com",
               Tags = new[] { "developer", "devops" }, Profile = new UserProfile { Score = 91, Level = "senior" } },
    new User { Name = "Frank", Age = 45, City = "Boston", Email = "frank@example.com",
               Tags = new[] { "architect" }, Profile = new UserProfile { Score = 98, Level = "principal" } },
});
Console.WriteLine($"    Inserted count: {manyResult.InsertedCount}");

// Find All
Console.WriteLine("\n>>> Find (all)");
var allUsers = users.Find();
Console.WriteLine($"    Total users: {allUsers.Count}");
foreach (var u in allUsers)
    Console.WriteLine($"    - {u.Name}, {u.Age}, {u.City}");

// Find One
Console.WriteLine("\n>>> FindOne");
var alice = users.FindOne(Builders<User>.Filter.Eq("Name", "Alice"));
Console.WriteLine($"    Found: {alice?.Name}, Age: {alice?.Age}");

// Count Documents
Console.WriteLine("\n>>> CountDocuments");
var count = users.CountDocuments();
Console.WriteLine($"    Total documents: {count}");

// ============================================================
// 2. QUERY OPERATORS
// ============================================================
PrintSection("2. QUERY OPERATORS");

// Comparison operators: $gt, $gte, $lt, $lte
Console.WriteLine(">>> Age >= 30 (using $gte)");
var seniorUsers = users.Find(Builders<User>.Filter.Gte("Age", 30));
PrintUsers(seniorUsers);

Console.WriteLine("\n>>> Age < 30 (using $lt)");
var youngUsers = users.Find(Builders<User>.Filter.Lt("Age", 30));
PrintUsers(youngUsers);

// $ne - Not Equal
Console.WriteLine("\n>>> City != 'New York' (using $ne)");
var notNYUsers = users.Find(Builders<User>.Filter.Ne("City", "New York"));
PrintUsers(notNYUsers);

// $in - In array
Console.WriteLine("\n>>> City in ['New York', 'Boston'] (using $in)");
var nyOrBoston = users.Find(Builders<User>.Filter.In("City", "New York", "Boston"));
PrintUsers(nyOrBoston);

// $exists - Field exists
Console.WriteLine("\n>>> Email exists (using $exists)");
var withEmail = users.Find(Builders<User>.Filter.Exists("Email"));
Console.WriteLine($"    Users with email: {withEmail.Count}");

// $regex - Regex match
Console.WriteLine("\n>>> Name starts with 'A' or 'B' (using $regex)");
var abNames = users.Find(Builders<User>.Filter.Regex("Name", "^[AB]"));
PrintUsers(abNames);

// Logical operators: $and, $or
Console.WriteLine("\n>>> (Age >= 30 AND City = 'New York') using $and");
var andFilter = Builders<User>.Filter.And(
    Builders<User>.Filter.Gte("Age", 30),
    Builders<User>.Filter.Eq("City", "New York")
);
var seniorNY = users.Find(andFilter);
PrintUsers(seniorNY);

Console.WriteLine("\n>>> (City = 'Chicago' OR City = 'Boston') using $or");
var orFilter = Builders<User>.Filter.Or(
    Builders<User>.Filter.Eq("City", "Chicago"),
    Builders<User>.Filter.Eq("City", "Boston")
);
var chicagoOrBoston = users.Find(orFilter);
PrintUsers(chicagoOrBoston);

// Using JSON filter directly
Console.WriteLine("\n>>> Using raw JSON filter: Profile.Score > 90");
var highScorers = users.Find("{\"Profile.Score\": {\"$gt\": 90}}");
PrintUsers(highScorers);

// ============================================================
// 3. UPDATE OPERATORS
// ============================================================
PrintSection("3. UPDATE OPERATORS");

// $set - Set field value
Console.WriteLine(">>> $set: Update Alice's age to 31");
var updateResult = users.UpdateOne(
    Builders<User>.Filter.Eq("Name", "Alice"),
    Builders<User>.Update.Set("Age", 31)
);
Console.WriteLine($"    Matched: {updateResult.MatchedCount}, Modified: {updateResult.ModifiedCount}");

// $inc - Increment
Console.WriteLine("\n>>> $inc: Increment Bob's Profile.Score by 5");
users.UpdateOne(
    Builders<User>.Filter.Eq("Name", "Bob"),
    Builders<User>.Update.Inc("Profile.Score", 5)
);
var bob = users.FindOne(Builders<User>.Filter.Eq("Name", "Bob"));
Console.WriteLine($"    Bob's new score: {bob?.Profile?.Score}");

// $push - Add to array
Console.WriteLine("\n>>> $push: Add 'speaker' tag to Carol");
users.UpdateOne(
    Builders<User>.Filter.Eq("Name", "Carol"),
    Builders<User>.Update.Push("Tags", "speaker")
);
var carol = users.FindOne(Builders<User>.Filter.Eq("Name", "Carol"));
Console.WriteLine($"    Carol's tags: [{string.Join(", ", carol?.Tags ?? Array.Empty<string>())}]");

// $pull - Remove from array
Console.WriteLine("\n>>> $pull: Remove 'intern' tag from David");
users.UpdateOne(
    Builders<User>.Filter.Eq("Name", "David"),
    Builders<User>.Update.Pull("Tags", "intern")
);
var david = users.FindOne(Builders<User>.Filter.Eq("Name", "David"));
Console.WriteLine($"    David's tags: [{string.Join(", ", david?.Tags ?? Array.Empty<string>())}]");

// $addToSet - Add unique value to array
Console.WriteLine("\n>>> $addToSet: Add 'mentor' to Eve (unique only)");
users.UpdateOne(
    Builders<User>.Filter.Eq("Name", "Eve"),
    Builders<User>.Update.AddToSet("Tags", "mentor")
);
// Try adding again - should not duplicate
users.UpdateOne(
    Builders<User>.Filter.Eq("Name", "Eve"),
    Builders<User>.Update.AddToSet("Tags", "mentor")
);
var eve = users.FindOne(Builders<User>.Filter.Eq("Name", "Eve"));
Console.WriteLine($"    Eve's tags: [{string.Join(", ", eve?.Tags ?? Array.Empty<string>())}]");

// UpdateMany
Console.WriteLine("\n>>> UpdateMany: Add 'verified' status to all NY users");
var updateManyResult = users.UpdateMany(
    Builders<User>.Filter.Eq("City", "New York"),
    Builders<User>.Update.Set("Verified", true)
);
Console.WriteLine($"    Matched: {updateManyResult.MatchedCount}, Modified: {updateManyResult.ModifiedCount}");

// ============================================================
// 4. FIND OPTIONS (Projection, Sort, Limit, Skip)
// ============================================================
PrintSection("4. FIND OPTIONS");

// Sort
Console.WriteLine(">>> Sort by Age ascending");
var sortedByAge = users.Find(
    Builders<User>.Filter.Empty,
    new FindOptions { Sort = new List<(string, int)> { ("Age", 1) } }
);
foreach (var u in sortedByAge)
    Console.WriteLine($"    {u.Name}: {u.Age}");

Console.WriteLine("\n>>> Sort by City asc, Age desc");
var multiSort = users.Find(
    Builders<User>.Filter.Empty,
    new FindOptions { Sort = new List<(string, int)> { ("City", 1), ("Age", -1) } }
);
foreach (var u in multiSort)
    Console.WriteLine($"    {u.Name}: {u.City}, {u.Age}");

// Limit & Skip (Pagination)
Console.WriteLine("\n>>> Pagination: Skip 2, Limit 2");
var page = users.Find(
    Builders<User>.Filter.Empty,
    new FindOptions { Skip = 2, Limit = 2 }
);
Console.WriteLine($"    Results: {page.Count}");
foreach (var u in page)
    Console.WriteLine($"    - {u.Name}");

// Projection
Console.WriteLine("\n>>> Projection: Only Name and City");
var projected = users.Find(
    Builders<User>.Filter.Empty,
    new FindOptions
    {
        Projection = new Dictionary<string, int> { { "Name", 1 }, { "City", 1 }, { "_id", 0 } },
        Limit = 3
    }
);
Console.WriteLine($"    Returned {projected.Count} documents with limited fields");

// ============================================================
// 5. INDEXING
// ============================================================
PrintSection("5. INDEXING");

// Create single field index
Console.WriteLine(">>> Create index on 'Email' (unique)");
var emailIdx = users.CreateIndex("Email", unique: true);
Console.WriteLine($"    Created: {emailIdx}");

Console.WriteLine("\n>>> Create index on 'Age'");
var ageIdx = users.CreateIndex("Age");
Console.WriteLine($"    Created: {ageIdx}");

// Create compound index
Console.WriteLine("\n>>> Create compound index on ['City', 'Age']");
var compoundIdx = users.CreateCompoundIndex(new[] { "City", "Age" });
Console.WriteLine($"    Created: {compoundIdx}");

// List indexes
Console.WriteLine("\n>>> List all indexes");
var indexes = users.ListIndexes();
foreach (var idx in indexes)
    Console.WriteLine($"    - {idx}");

// Explain query
Console.WriteLine("\n>>> Explain query: Age = 30");
var plan = users.Explain(Builders<User>.Filter.Eq("Age", 30));
Console.WriteLine($"    Query plan: {plan}");

// ============================================================
// 6. AGGREGATION PIPELINE
// ============================================================
PrintSection("6. AGGREGATION PIPELINE");

// Simple aggregation: Group by city
Console.WriteLine(">>> Group users by City, count and avg age");
var cityStats = users.Aggregate<CityStats>(@"[
    { ""$group"": {
        ""_id"": ""$City"",
        ""count"": { ""$sum"": 1 },
        ""avgAge"": { ""$avg"": ""$Age"" },
        ""maxAge"": { ""$max"": ""$Age"" }
    }},
    { ""$sort"": { ""count"": -1 } }
]");
foreach (var stat in cityStats)
    Console.WriteLine($"    {stat._id}: {stat.count} users, avg age: {stat.avgAge?.ToString("F1") ?? "N/A"}, max: {stat.maxAge?.ToString("F0") ?? "N/A"}");

// Pipeline with $match, $group, $project
Console.WriteLine("\n>>> Pipeline: Match senior (Age >= 30), group by City");
var seniorStats = users.Aggregate<CityStats>(@"[
    { ""$match"": { ""Age"": { ""$gte"": 30 } } },
    { ""$group"": {
        ""_id"": ""$City"",
        ""count"": { ""$sum"": 1 },
        ""avgScore"": { ""$avg"": ""$Profile.Score"" }
    }},
    { ""$sort"": { ""avgScore"": -1 } },
    { ""$limit"": 3 }
]");
foreach (var stat in seniorStats)
    Console.WriteLine($"    {stat._id}: {stat.count} senior users, avg score: {stat.avgScore?.ToString("F1") ?? "N/A"}");

// ============================================================
// 7. NESTED DOCUMENTS (Dot Notation)
// ============================================================
PrintSection("7. NESTED DOCUMENTS (Dot Notation)");

// Create a collection with deeply nested documents
var companies = client.GetCollection<Company>("companies");

Console.WriteLine(">>> Insert companies with nested documents");
companies.InsertMany(new[]
{
    new Company
    {
        Name = "TechCorp",
        Location = new Location
        {
            Country = "USA",
            City = "San Francisco",
            Address = new Address { Street = "123 Tech Blvd", Zip = "94105" }
        },
        Stats = new CompanyStats { Employees = 500, Revenue = 50000000, Rating = 4.5 }
    },
    new Company
    {
        Name = "DataSoft",
        Location = new Location
        {
            Country = "USA",
            City = "New York",
            Address = new Address { Street = "456 Data Ave", Zip = "10001" }
        },
        Stats = new CompanyStats { Employees = 200, Revenue = 20000000, Rating = 4.2 }
    },
    new Company
    {
        Name = "CloudNet",
        Location = new Location
        {
            Country = "Germany",
            City = "Berlin",
            Address = new Address { Street = "789 Cloud Str", Zip = "10115" }
        },
        Stats = new CompanyStats { Employees = 150, Revenue = 15000000, Rating = 4.8 }
    },
    new Company
    {
        Name = "AILabs",
        Location = new Location
        {
            Country = "USA",
            City = "Boston",
            Address = new Address { Street = "321 AI Road", Zip = "02101" }
        },
        Stats = new CompanyStats { Employees = 300, Revenue = 35000000, Rating = 4.6 }
    }
});
Console.WriteLine("    Inserted 4 companies with nested location and stats");

// Query nested field with dot notation
Console.WriteLine("\n>>> Query: Location.Country = 'USA' (dot notation)");
var usCompanies = companies.Find("{\"Location.Country\": \"USA\"}");
foreach (var c in usCompanies)
    Console.WriteLine($"    - {c.Name} in {c.Location?.City}");

Console.WriteLine("\n>>> Query: Location.City = 'New York'");
var nyCompanies = companies.Find("{\"Location.City\": \"New York\"}");
foreach (var c in nyCompanies)
    Console.WriteLine($"    - {c.Name}: {c.Location?.Address?.Street}");

// Query deeply nested field
Console.WriteLine("\n>>> Query: Location.Address.Zip starts with '10' (regex on nested)");
var zip10Companies = companies.Find("{\"Location.Address.Zip\": {\"$regex\": \"^10\"}}");
foreach (var c in zip10Companies)
    Console.WriteLine($"    - {c.Name}: ZIP {c.Location?.Address?.Zip}");

// Query nested numeric field
Console.WriteLine("\n>>> Query: Stats.Employees >= 200");
var largeCompanies = companies.Find("{\"Stats.Employees\": {\"$gte\": 200}}");
foreach (var c in largeCompanies)
    Console.WriteLine($"    - {c.Name}: {c.Stats?.Employees} employees");

Console.WriteLine("\n>>> Query: Stats.Rating > 4.5");
var highRated = companies.Find("{\"Stats.Rating\": {\"$gt\": 4.5}}");
foreach (var c in highRated)
    Console.WriteLine($"    - {c.Name}: rating {c.Stats?.Rating}");

// Update nested field with dot notation
Console.WriteLine("\n>>> Update: Set TechCorp's Stats.Rating to 4.9");
companies.UpdateOne(
    Builders<Company>.Filter.Eq("Name", "TechCorp"),
    "{\"$set\": {\"Stats.Rating\": 4.9}}"
);
var techCorp = companies.FindOne(Builders<Company>.Filter.Eq("Name", "TechCorp"));
Console.WriteLine($"    TechCorp new rating: {techCorp?.Stats?.Rating}");

// Update deeply nested field
Console.WriteLine("\n>>> Update: Change DataSoft's Location.Address.Street");
companies.UpdateOne(
    Builders<Company>.Filter.Eq("Name", "DataSoft"),
    "{\"$set\": {\"Location.Address.Street\": \"789 New Data Plaza\"}}"
);
var dataSoft = companies.FindOne(Builders<Company>.Filter.Eq("Name", "DataSoft"));
Console.WriteLine($"    DataSoft new address: {dataSoft?.Location?.Address?.Street}");

// Increment nested numeric field
Console.WriteLine("\n>>> Update: Increment CloudNet's Stats.Employees by 50");
companies.UpdateOne(
    Builders<Company>.Filter.Eq("Name", "CloudNet"),
    "{\"$inc\": {\"Stats.Employees\": 50}}"
);
var cloudNet = companies.FindOne(Builders<Company>.Filter.Eq("Name", "CloudNet"));
Console.WriteLine($"    CloudNet employees: {cloudNet?.Stats?.Employees}");

// DEBUG: Print raw document JSON
Console.WriteLine("\n>>> DEBUG: Raw document structure");
var allCompanies = companies.Find();
Console.WriteLine($"    First company serialized: {JsonSerializer.Serialize(allCompanies[0])}");

// Aggregation with nested fields
Console.WriteLine("\n>>> Aggregation: Group by Location.Country, sum employees");
// DEBUG: Try raw aggregation to see what Rust returns
var rawAggPipeline = @"[
    { ""$group"": {
        ""_id"": ""$Location.Country"",
        ""totalEmployees"": { ""$sum"": ""$Stats.Employees"" },
        ""avgRating"": { ""$avg"": ""$Stats.Rating"" },
        ""companyCount"": { ""$sum"": 1 }
    }},
    { ""$sort"": { ""totalEmployees"": -1 } }
]";
Console.WriteLine($"    Pipeline: {rawAggPipeline}");
var countryStats = companies.Aggregate<CountryStats>(rawAggPipeline);
Console.WriteLine($"    Raw result: {JsonSerializer.Serialize(countryStats)}");
foreach (var s in countryStats)
    Console.WriteLine($"    {s._id}: {s.companyCount} companies, {s.totalEmployees} employees, avg rating: {s.avgRating?.ToString("F2") ?? "N/A"}");

// Sort by nested field
Console.WriteLine("\n>>> Sort by Stats.Revenue descending");
var byRevenue = companies.Find(
    Builders<Company>.Filter.Empty,
    new FindOptions { Sort = new List<(string, int)> { ("Stats.Revenue", -1) } }
);
foreach (var c in byRevenue)
    Console.WriteLine($"    - {c.Name}: ${c.Stats?.Revenue:N0}");

// ============================================================
// 8. TRANSACTIONS
// ============================================================
PrintSection("8. TRANSACTIONS");

var accounts = client.GetCollection<Account>("accounts");

// Setup accounts
accounts.InsertMany(new[]
{
    new Account { AccountId = "A001", Owner = "Alice", Balance = 1000 },
    new Account { AccountId = "A002", Owner = "Bob", Balance = 500 }
});

Console.WriteLine(">>> Initial balances:");
PrintAccounts(accounts);

// Begin transaction
Console.WriteLine("\n>>> Begin transaction: Transfer 200 from Alice to Bob");
var txId = client.BeginTransaction();
Console.WriteLine($"    Transaction ID: {txId}");

try
{
    // Deduct from Alice
    accounts.UpdateOne(
        Builders<Account>.Filter.Eq("AccountId", "A001"),
        Builders<Account>.Update.Inc("Balance", -200)
    );

    // Add to Bob
    accounts.UpdateOne(
        Builders<Account>.Filter.Eq("AccountId", "A002"),
        Builders<Account>.Update.Inc("Balance", 200)
    );

    // Commit
    client.CommitTransaction(txId);
    Console.WriteLine("    Transaction committed!");
}
catch (Exception ex)
{
    client.RollbackTransaction(txId);
    Console.WriteLine($"    Transaction rolled back: {ex.Message}");
}

Console.WriteLine("\n>>> Final balances:");
PrintAccounts(accounts);

// ============================================================
// 9. DATABASE OPERATIONS
// ============================================================
PrintSection("9. DATABASE OPERATIONS");

// List collections
Console.WriteLine(">>> List collections");
var collections = client.ListCollections();
foreach (var col in collections)
    Console.WriteLine($"    - {col}");

// Get stats
Console.WriteLine("\n>>> Database statistics");
var stats = client.GetStats();
Console.WriteLine($"    {stats}");

// Distinct values
Console.WriteLine("\n>>> Distinct cities");
var cities = users.Distinct<string>("City");
Console.WriteLine($"    Cities: [{string.Join(", ", cities)}]");

// ============================================================
// 10. DELETE OPERATIONS
// ============================================================
PrintSection("10. DELETE OPERATIONS");

Console.WriteLine($">>> Users before delete: {users.CountDocuments()}");

// Delete one
Console.WriteLine("\n>>> DeleteOne: Remove David");
var deleteResult = users.DeleteOne(Builders<User>.Filter.Eq("Name", "David"));
Console.WriteLine($"    Deleted: {deleteResult.DeletedCount}");

// Delete many
Console.WriteLine("\n>>> DeleteMany: Remove users with Age < 30");
var deleteManyResult = users.DeleteMany(Builders<User>.Filter.Lt("Age", 30));
Console.WriteLine($"    Deleted: {deleteManyResult.DeletedCount}");

Console.WriteLine($"\n>>> Users after delete: {users.CountDocuments()}");

// ============================================================
// 11. CLEANUP & COMPACTION
// ============================================================
PrintSection("11. CLEANUP & COMPACTION");

Console.WriteLine(">>> Compact database (remove tombstones)");
var compactResult = client.Compact();
Console.WriteLine($"    Size before: {compactResult.SizeBefore} bytes");
Console.WriteLine($"    Size after: {compactResult.SizeAfter} bytes");
Console.WriteLine($"    Tombstones removed: {compactResult.TombstonesRemoved}");

Console.WriteLine("\n>>> Flush to disk");
client.Flush();
Console.WriteLine("    Data flushed successfully!");

// Drop collection
Console.WriteLine("\n>>> Drop 'accounts' collection");
client.DropCollection("accounts");
Console.WriteLine($"    Collections remaining: [{string.Join(", ", client.ListCollections())}]");

// ============================================================
// DONE
// ============================================================
Console.WriteLine();
Console.WriteLine("╔══════════════════════════════════════════════════════════╗");
Console.WriteLine("║                    Demo Complete!                        ║");
Console.WriteLine("╚══════════════════════════════════════════════════════════╝");

// Cleanup
File.Delete(dbPath);
if (File.Exists(dbPath + ".wal")) File.Delete(dbPath + ".wal");

// ============================================================
// Helper Methods & Models
// ============================================================

static void PrintSection(string title)
{
    Console.WriteLine();
    Console.WriteLine($"──────────────────────────────────────────────────────────");
    Console.WriteLine($"  {title}");
    Console.WriteLine($"──────────────────────────────────────────────────────────");
}

static void PrintUsers(List<User> users)
{
    Console.WriteLine($"    Found {users.Count} users:");
    foreach (var u in users)
        Console.WriteLine($"    - {u.Name}, {u.Age}, {u.City}");
}

static void PrintAccounts(IronBaseCollection<Account> accounts)
{
    var all = accounts.Find();
    foreach (var a in all)
        Console.WriteLine($"    {a.Owner} ({a.AccountId}): ${a.Balance}");
}

// Document Models
public class User
{
    [JsonPropertyName("_id")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public object? Id { get; set; }
    public string Name { get; set; } = "";
    public int Age { get; set; }
    public string City { get; set; } = "";
    public string? Email { get; set; }
    public string[]? Tags { get; set; }
    public UserProfile? Profile { get; set; }
    public bool Verified { get; set; }
}

public class UserProfile
{
    public int Score { get; set; }
    public string Level { get; set; } = "";
}

public class Account
{
    [JsonPropertyName("_id")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public object? Id { get; set; }
    public string AccountId { get; set; } = "";
    public string Owner { get; set; } = "";
    public decimal Balance { get; set; }
}

public class CityStats
{
    public object? _id { get; set; }
    public double count { get; set; }
    public double? avgAge { get; set; }
    public double? maxAge { get; set; }
    public double? avgScore { get; set; }
}

// Nested document models for Section 7
public class Company
{
    [JsonPropertyName("_id")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public object? Id { get; set; }
    public string Name { get; set; } = "";
    public Location? Location { get; set; }
    public CompanyStats? Stats { get; set; }
}

public class Location
{
    public string Country { get; set; } = "";
    public string City { get; set; } = "";
    public Address? Address { get; set; }
}

public class Address
{
    public string Street { get; set; } = "";
    public string Zip { get; set; } = "";
}

public class CompanyStats
{
    public int Employees { get; set; }
    public decimal Revenue { get; set; }
    public double Rating { get; set; }
}

public class CountryStats
{
    public object? _id { get; set; }
    public double? totalEmployees { get; set; }
    public double? avgRating { get; set; }
    public double? companyCount { get; set; }
}
