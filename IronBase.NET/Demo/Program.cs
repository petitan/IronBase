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

// $project stage - Reshape documents
Console.WriteLine("\n>>> $project: Rename fields and exclude _id");
var projectedUsers = users.Aggregate<ProjectedUser>(@"[
    { ""$project"": {
        ""_id"": 0,
        ""fullName"": ""$Name"",
        ""years"": ""$Age"",
        ""location"": ""$City""
    }},
    { ""$limit"": 3 }
]");
foreach (var pu in projectedUsers)
    Console.WriteLine($"    {pu.fullName}: {pu.years} years old, from {pu.location}");

// $skip stage - Pagination
Console.WriteLine("\n>>> $skip + $limit: Skip first 2, get next 2 (pagination)");
var skipped = users.Aggregate<CityStats>(@"[
    { ""$sort"": { ""Age"": 1 } },
    { ""$skip"": 2 },
    { ""$limit"": 2 },
    { ""$project"": { ""_id"": ""$Name"", ""count"": ""$Age"" } }
]");
foreach (var s in skipped)
    Console.WriteLine($"    {s._id}: age {s.count}");

// $min/$max accumulators
Console.WriteLine("\n>>> $min/$max: Min and max age per city");
var minMaxStats = users.Aggregate<MinMaxStats>(@"[
    { ""$group"": {
        ""_id"": ""$City"",
        ""minAge"": { ""$min"": ""$Age"" },
        ""maxAge"": { ""$max"": ""$Age"" },
        ""avgAge"": { ""$avg"": ""$Age"" }
    }},
    { ""$sort"": { ""_id"": 1 } }
]");
foreach (var m in minMaxStats)
    Console.WriteLine($"    {m._id}: min={m.minAge}, max={m.maxAge}, avg={m.avgAge?.ToString("F1") ?? "N/A"}");

// $first/$last accumulators
Console.WriteLine("\n>>> $first/$last: First and last user name per city (sorted by age)");
var firstLastStats = users.Aggregate<FirstLastStats>(@"[
    { ""$sort"": { ""Age"": 1 } },
    { ""$group"": {
        ""_id"": ""$City"",
        ""youngest"": { ""$first"": ""$Name"" },
        ""oldest"": { ""$last"": ""$Name"" },
        ""count"": { ""$sum"": 1 }
    }},
    { ""$sort"": { ""count"": -1 } }
]");
foreach (var f in firstLastStats)
    Console.WriteLine($"    {f._id}: youngest={f.youngest}, oldest={f.oldest} ({f.count} users)");

// Complex multi-stage pipeline
Console.WriteLine("\n>>> Complex pipeline: $match -> $group -> $sort -> $skip -> $limit");
var complexPipeline = users.Aggregate<ComplexStats>(@"[
    { ""$match"": { ""Profile.Score"": { ""$gte"": 75 } } },
    { ""$group"": {
        ""_id"": ""$Profile.Level"",
        ""userCount"": { ""$sum"": 1 },
        ""avgScore"": { ""$avg"": ""$Profile.Score"" },
        ""minScore"": { ""$min"": ""$Profile.Score"" },
        ""maxScore"": { ""$max"": ""$Profile.Score"" }
    }},
    { ""$sort"": { ""avgScore"": -1 } },
    { ""$skip"": 0 },
    { ""$limit"": 5 }
]");
foreach (var c in complexPipeline)
    Console.WriteLine($"    {c._id}: {c.userCount} users, avg={c.avgScore?.ToString("F1") ?? "N/A"}, min={c.minScore?.ToString("F0") ?? "N/A"}, max={c.maxScore?.ToString("F0") ?? "N/A"}");

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

// Create index on nested field
Console.WriteLine("\n>>> Create index on nested field 'Stats.Rating'");
var ratingIdx = companies.CreateIndex("Stats.Rating");
Console.WriteLine($"    Created: {ratingIdx}");

Console.WriteLine("\n>>> Create index on deeply nested 'Location.Address.Zip'");
var zipIdx = companies.CreateIndex("Location.Address.Zip");
Console.WriteLine($"    Created: {zipIdx}");

// List indexes on companies
Console.WriteLine("\n>>> List indexes on companies collection");
var companyIndexes = companies.ListIndexes();
foreach (var idx in companyIndexes)
    Console.WriteLine($"    - {idx}");

// Explain query with nested field index
Console.WriteLine("\n>>> Explain query with nested field index: Stats.Rating > 4.5");
var nestedExplain = companies.Explain("{\"Stats.Rating\": {\"$gt\": 4.5}}");
Console.WriteLine($"    Plan: {nestedExplain}");

// ============================================================
// 8. ADVANCED ARRAY OPERATORS
// ============================================================
PrintSection("8. ADVANCED ARRAY OPERATORS");

var products = client.GetCollection<Product>("products");

Console.WriteLine(">>> Insert products with tags array");
products.InsertMany(new[]
{
    new Product { Name = "Laptop Pro", Price = 1299, Tags = new[] { "electronics", "computer", "portable" }, Ratings = new[] { 5, 4, 5, 5, 4 } },
    new Product { Name = "Wireless Mouse", Price = 49, Tags = new[] { "electronics", "accessory", "wireless" }, Ratings = new[] { 4, 4, 3, 5 } },
    new Product { Name = "USB Hub", Price = 29, Tags = new[] { "electronics", "accessory", "usb" }, Ratings = new[] { 3, 4, 4 } },
    new Product { Name = "Desk Lamp", Price = 79, Tags = new[] { "furniture", "lighting", "office" }, Ratings = new[] { 5, 5, 5 } },
    new Product { Name = "Gaming Chair", Price = 399, Tags = new[] { "furniture", "gaming", "ergonomic" }, Ratings = new[] { 4, 5, 4, 5 } },
});
Console.WriteLine("    Inserted 5 products");

// $all - Match documents where array contains ALL specified values
Console.WriteLine("\n>>> $all: Products with tags containing BOTH 'electronics' AND 'accessory'");
var allMatch = products.Find("{\"Tags\": {\"$all\": [\"electronics\", \"accessory\"]}}");
foreach (var p in allMatch)
    Console.WriteLine($"    - {p.Name}: [{string.Join(", ", p.Tags ?? Array.Empty<string>())}]");

// $size - Match documents where array has specific size
Console.WriteLine("\n>>> $size: Products with exactly 3 tags");
var size3 = products.Find("{\"Tags\": {\"$size\": 3}}");
foreach (var p in size3)
    Console.WriteLine($"    - {p.Name}: {p.Tags?.Length} tags");

// $elemMatch - Match documents where array element matches multiple conditions
Console.WriteLine("\n>>> $elemMatch: Products with a rating that equals 5");
var has5Rating = products.Find("{\"Ratings\": {\"$elemMatch\": {\"$eq\": 5}}}");
foreach (var p in has5Rating)
    Console.WriteLine($"    - {p.Name}: ratings [{string.Join(", ", p.Ratings ?? Array.Empty<int>())}]");

// $nin - Not in
Console.WriteLine("\n>>> $nin: Products without 'furniture' or 'gaming' tags");
var notFurniture = products.Find("{\"Tags\": {\"$nin\": [\"furniture\", \"gaming\"]}}");
foreach (var p in notFurniture)
    Console.WriteLine($"    - {p.Name}");

// $pop - Remove last element from array
Console.WriteLine("\n>>> $pop: Remove last rating from Laptop Pro");
products.UpdateOne(
    Builders<Product>.Filter.Eq("Name", "Laptop Pro"),
    "{\"$pop\": {\"Ratings\": 1}}"
);
var laptop = products.FindOne(Builders<Product>.Filter.Eq("Name", "Laptop Pro"));
Console.WriteLine($"    Laptop Pro ratings: [{string.Join(", ", laptop?.Ratings ?? Array.Empty<int>())}]");

// ============================================================
// 9. ADDITIONAL QUERY PATTERNS
// ============================================================
PrintSection("9. ADDITIONAL QUERY PATTERNS");

// Combined nested field queries
Console.WriteLine(">>> Complex nested query: USA companies with >200 employees and rating >4.0");
var complexQuery = companies.Find(@"{
    ""$and"": [
        {""Location.Country"": ""USA""},
        {""Stats.Employees"": {""$gt"": 200}},
        {""Stats.Rating"": {""$gt"": 4.0}}
    ]
}");
foreach (var c in complexQuery)
    Console.WriteLine($"    - {c.Name}: {c.Stats?.Employees} employees, rating {c.Stats?.Rating}");

// Range query on nested field
Console.WriteLine("\n>>> Range query: Revenue between 20M and 40M");
var revenueRange = companies.Find(@"{
    ""$and"": [
        {""Stats.Revenue"": {""$gte"": 20000000}},
        {""Stats.Revenue"": {""$lte"": 40000000}}
    ]
}");
foreach (var c in revenueRange)
    Console.WriteLine($"    - {c.Name}: ${c.Stats?.Revenue:N0}");

// Not operator with nested field
Console.WriteLine("\n>>> $not: Companies NOT in Germany");
var notGermany = companies.Find(@"{
    ""Location.Country"": {""$not"": {""$eq"": ""Germany""}}
}");
foreach (var c in notGermany)
    Console.WriteLine($"    - {c.Name} ({c.Location?.Country})");

// Projection with nested fields
Console.WriteLine("\n>>> Projection: Only Name and nested Stats.Rating");
var projectedCompanies = companies.Find(
    Builders<Company>.Filter.Empty,
    new FindOptions
    {
        Projection = new Dictionary<string, int> { { "Name", 1 }, { "Stats.Rating", 1 }, { "_id", 0 } },
        Limit = 3
    }
);
Console.WriteLine($"    Returned {projectedCompanies.Count} documents with projected fields");

// ============================================================
// 10. PERSISTENCE DEMO
// ============================================================
PrintSection("10. PERSISTENCE DEMO");

// Create a separate database to demonstrate persistence
var persistPath = "persist_demo.mlite";
if (File.Exists(persistPath)) File.Delete(persistPath);
if (File.Exists(persistPath + ".wal")) File.Delete(persistPath + ".wal");

Console.WriteLine(">>> Session 1: Create database and insert data");
{
    using var persistClient = new IronBaseClient(persistPath);
    var persistUsers = persistClient.GetCollection<User>("demo_users");

    // Create index on nested field
    persistUsers.CreateIndex("Profile.Score");
    Console.WriteLine("    Created index on Profile.Score");

    // Insert users with nested profiles
    persistUsers.InsertMany(new[]
    {
        new User { Name = "Alice", Age = 30, City = "NYC", Profile = new UserProfile { Score = 95, Level = "senior" } },
        new User { Name = "Bob", Age = 25, City = "LA", Profile = new UserProfile { Score = 82, Level = "mid" } },
        new User { Name = "Carol", Age = 35, City = "NYC", Profile = new UserProfile { Score = 91, Level = "senior" } },
    });
    Console.WriteLine("    Inserted 3 users with nested profiles");

    // Verify data
    var persistCount1 = persistUsers.CountDocuments();
    Console.WriteLine($"    Total users: {persistCount1}");

    // Flush to ensure data is written
    persistClient.Flush();
    Console.WriteLine("    Data flushed to disk");
}

Console.WriteLine("\n>>> Session 2: Reopen database and verify data persisted");
{
    using var persistClient = new IronBaseClient(persistPath);
    var persistUsers = persistClient.GetCollection<User>("demo_users");

    // Check document count
    var persistCount2 = persistUsers.CountDocuments();
    Console.WriteLine($"    Documents found after reopen: {persistCount2}");

    // Verify indexes persisted
    var persistIndexes = persistUsers.ListIndexes();
    Console.WriteLine($"    Indexes: [{string.Join(", ", persistIndexes)}]");

    // Query using nested field index
    Console.WriteLine("\n    Query: Profile.Score >= 90 (using persisted index)");
    var persistHighScorers = persistUsers.Find("{\"Profile.Score\": {\"$gte\": 90}}");
    foreach (var u in persistHighScorers)
        Console.WriteLine($"      - {u.Name}: score {u.Profile?.Score}");

    // Verify explain shows index usage
    var persistExplain = persistUsers.Explain("{\"Profile.Score\": {\"$gte\": 90}}");
    Console.WriteLine($"\n    Explain: {persistExplain}");
}

// Cleanup persistence demo
File.Delete(persistPath);
if (File.Exists(persistPath + ".wal")) File.Delete(persistPath + ".wal");
// Delete index files
foreach (var f in Directory.GetFiles(".", "persist_demo*.idx"))
    File.Delete(f);
Console.WriteLine("\n    Persistence demo cleanup complete");

// ============================================================
// 11. TRANSACTIONS
// ============================================================
PrintSection("11. TRANSACTIONS");

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
// 12. DATABASE OPERATIONS
// ============================================================
PrintSection("12. DATABASE OPERATIONS");

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
// 13. DELETE OPERATIONS
// ============================================================
PrintSection("13. DELETE OPERATIONS");

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
// 14. CLEANUP & COMPACTION
// ============================================================
PrintSection("14. CLEANUP & COMPACTION");

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

// Product model for array operators demo
public class Product
{
    [JsonPropertyName("_id")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public object? Id { get; set; }
    public string Name { get; set; } = "";
    public decimal Price { get; set; }
    public string[]? Tags { get; set; }
    public int[]? Ratings { get; set; }
}

// Additional aggregation result models
public class ProjectedUser
{
    public string? fullName { get; set; }
    public double? years { get; set; }
    public string? location { get; set; }
}

public class MinMaxStats
{
    public object? _id { get; set; }
    public double? minAge { get; set; }
    public double? maxAge { get; set; }
    public double? avgAge { get; set; }
}

public class FirstLastStats
{
    public object? _id { get; set; }
    public string? youngest { get; set; }
    public string? oldest { get; set; }
    public double? count { get; set; }
}

public class ComplexStats
{
    public object? _id { get; set; }
    public double? userCount { get; set; }
    public double? avgScore { get; set; }
    public double? minScore { get; set; }
    public double? maxScore { get; set; }
}
