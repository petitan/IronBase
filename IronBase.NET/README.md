# IronBase.NET

[![NuGet](https://img.shields.io/nuget/v/IronBase)](https://www.nuget.org/packages/IronBase/)
[![.NET](https://img.shields.io/badge/.NET-8.0-blue)](https://dotnet.microsoft.com/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](../LICENSE)

.NET bindings for IronBase - a lightweight embedded NoSQL document database with a MongoDB-like API, powered by Rust.

## Features

- **MongoDB-like API**: Familiar `IronBaseClient`, `IronBaseCollection<T>`, and `Builders<T>` pattern
- **Embedded database**: Single-file, serverless, zero-configuration
- **Cross-platform**: Windows x64 and Linux x64 support
- **High performance**: Rust core with B+ tree indexing
- **ACD transactions**: Atomicity, Consistency, Durability (no isolation)
- **Type-safe**: Full C# generics support with strong typing

## Installation

### NuGet Package

```bash
dotnet add package IronBase
```

### Manual Installation

1. Build the native library:
```bash
cargo build --release -p ironbase-ffi
```

2. Copy to runtime folder:
```bash
# Linux
cp target/release/libironbase_ffi.so IronBase.NET/runtimes/linux-x64/native/

# Windows
copy target\release\ironbase_ffi.dll IronBase.NET\runtimes\win-x64\native\
```

## Quick Start

```csharp
using IronBase;

// Open database (creates file if not exists)
using var client = new IronBaseClient("mydata.mlite");

// Get typed collection
var users = client.GetCollection<User>("users");

// Insert documents
users.InsertOne(new User { Name = "Alice", Age = 30 });
users.InsertMany(new[] {
    new User { Name = "Bob", Age = 25 },
    new User { Name = "Carol", Age = 35 }
});

// Query with filters
var filter = Builders<User>.Filter.Gte("Age", 25);
var adults = users.Find(filter);

// Update
var update = Builders<User>.Update.Set("Age", 31);
users.UpdateOne(Builders<User>.Filter.Eq("Name", "Alice"), update);

// Delete
users.DeleteOne(Builders<User>.Filter.Eq("Name", "Bob"));
```

## API Reference

### IronBaseClient

```csharp
// Open database
var client = new IronBaseClient("path/to/database.mlite");

// Get collection
var collection = client.GetCollection<T>("collectionName");

// List collections
var names = client.ListCollections();

// Drop collection
client.DropCollection("collectionName");

// Database statistics
var stats = client.Stats();

// Compact database (remove tombstones)
client.Compact();

// Force checkpoint (flush to disk)
client.Checkpoint();

// Always dispose when done
client.Dispose();
```

### IronBaseCollection&lt;T&gt;

#### CRUD Operations

```csharp
// Insert
string id = collection.InsertOne(document);
List<string> ids = collection.InsertMany(documents);

// Find
List<T> results = collection.Find(filter);
T? result = collection.FindOne(filter);
long count = collection.CountDocuments(filter);

// Update
UpdateResult result = collection.UpdateOne(filter, update);
UpdateResult result = collection.UpdateMany(filter, update);

// Delete
long deleted = collection.DeleteOne(filter);
long deleted = collection.DeleteMany(filter);
```

#### Cursor/Streaming (Large Datasets)

```csharp
// Create cursor with batch size
var cursor = collection.FindCursor(filter, batchSize: 500);

Console.WriteLine($"Total documents: {cursor.Total()}");

// Process in batches
while (!cursor.IsFinished())
{
    var batch = cursor.NextBatch();
    foreach (var doc in batch)
    {
        Process(doc);
    }
}

// Or one document at a time
cursor.Rewind();
while (cursor.MoveNext())
{
    var doc = cursor.Current;
    Process(doc);
}
```

### Filter Builders

```csharp
var filter = Builders<User>.Filter;

// Comparison
filter.Eq("Name", "Alice")           // Equal
filter.Ne("Status", "inactive")      // Not equal
filter.Gt("Age", 18)                 // Greater than
filter.Gte("Age", 18)                // Greater or equal
filter.Lt("Age", 65)                 // Less than
filter.Lte("Age", 65)                // Less or equal
filter.In("City", new[] { "NYC", "LA" })    // In array
filter.Nin("Status", new[] { "banned" })    // Not in array

// Logical
filter.And(filter.Eq("A", 1), filter.Eq("B", 2))
filter.Or(filter.Eq("A", 1), filter.Eq("B", 2))
filter.Not(filter.Eq("A", 1))

// Element
filter.Exists("email")               // Field exists
filter.Type("age", "number")         // Type check

// String
filter.Regex("name", "^A")           // Regex match

// Fuzzy (NEW in v1.0.5)
filter.Fuzzy("name", "john")         // Fuzzy match (default: jaro_winkler, 0.8)
filter.Fuzzy("name", "john", algorithm: "levenshtein", threshold: 0.7)
```

### Update Builders

```csharp
var update = Builders<User>.Update;

// Field updates
update.Set("Name", "Bob")            // Set field value
update.Inc("Score", 10)              // Increment number
update.Unset("TempField")            // Remove field

// Array updates
update.Push("Tags", "new_tag")       // Add to array
update.Pull("Tags", "old_tag")       // Remove from array
update.AddToSet("Tags", "unique")    // Add if not exists
update.Pop("Queue", 1)               // Remove last (-1 for first)

// Combine updates
update.Combine(
    update.Set("Name", "Bob"),
    update.Inc("Score", 10)
)
```

### Indexing

```csharp
// Create indexes
collection.CreateIndex("email", unique: true);
collection.CreateIndex("age");
collection.CreateCompoundIndex(new[] { "country", "city" });

// Create fuzzy index (NEW in v1.0.5)
collection.CreateFuzzyIndex("name");
collection.CreateFuzzyIndex("email", algorithm: "levenshtein", threshold: 0.7);

// List indexes
var indexes = collection.ListIndexes();

// Drop index
collection.DropIndex("users_age");

// Query plan analysis
var plan = collection.Explain(filter);
Console.WriteLine($"Plan: {plan.QueryPlan}, Index: {plan.IndexUsed}");
```

### Aggregation Pipeline

```csharp
var pipeline = new BsonDocument[]
{
    // $match - Filter documents
    new BsonDocument("$match", new BsonDocument("status", "completed")),

    // $group - Group and aggregate
    new BsonDocument("$group", new BsonDocument
    {
        { "_id", "$city" },
        { "totalRevenue", new BsonDocument("$sum", "$amount") },
        { "orderCount", new BsonDocument("$sum", 1) },
        { "avgOrder", new BsonDocument("$avg", "$amount") }
    }),

    // $project - Reshape output
    new BsonDocument("$project", new BsonDocument
    {
        { "city", "$_id" },
        { "revenue", "$totalRevenue" },
        { "_id", 0 }
    }),

    // $sort - Sort results
    new BsonDocument("$sort", new BsonDocument("revenue", -1)),

    // $limit - Limit results
    new BsonDocument("$limit", 10)
};

var results = collection.Aggregate(pipeline);
```

### Schema Validation

```csharp
// Set JSON schema
var schema = new BsonDocument
{
    { "type", "object" },
    { "required", new BsonArray { "name", "email" } },
    { "properties", new BsonDocument
        {
            { "name", new BsonDocument("type", "string") },
            { "email", new BsonDocument("type", "string") },
            { "age", new BsonDocument { { "type", "integer" }, { "minimum", 0 } } }
        }
    }
};
collection.SetSchema(schema);

// Get schema
var currentSchema = collection.GetSchema();

// Remove schema validation
collection.SetSchema(null);
```

### Error Handling

```csharp
try
{
    collection.InsertOne(document);
}
catch (IronBaseException ex) when (ex.Code == ErrorCode.DuplicateKey)
{
    Console.WriteLine("Duplicate key error");
}
catch (IronBaseException ex) when (ex.Code == ErrorCode.ValidationError)
{
    Console.WriteLine($"Schema validation failed: {ex.Message}");
}
catch (IronBaseException ex)
{
    Console.WriteLine($"Database error: {ex.Message}");
}
```

## Advanced Usage

### In-Memory Database

```csharp
// For testing - no file created
using var client = new IronBaseClient(":memory:");
var collection = client.GetCollection<User>("test");
// Data discarded when disposed
```

### Durability Modes

```csharp
// Safe mode (default) - Every write persisted immediately
var client = new IronBaseClient("data.mlite");

// Batch mode - Writes batched for performance
var client = new IronBaseClient("data.mlite", durability: DurabilityMode.Batch, batchSize: 100);

// Unsafe mode - Manual checkpoint required
var client = new IronBaseClient("data.mlite", durability: DurabilityMode.Unsafe);
client.Checkpoint(); // Manually flush to disk
```

### Transactions

```csharp
// Begin transaction
var txId = client.BeginTransaction();

try
{
    client.InsertOneTx("accounts", doc1, txId);
    client.UpdateOneTx("accounts", filter, update, txId);

    // Commit all changes
    client.CommitTransaction(txId);
}
catch
{
    // Rollback on error
    client.RollbackTransaction(txId);
    throw;
}
```

## Building from Source

### Prerequisites

- .NET 8.0 SDK
- Rust toolchain (for native library)

### Build Steps

```bash
# Build native library
cargo build --release -p ironbase-ffi

# Copy to runtime folder
cp target/release/libironbase_ffi.so IronBase.NET/runtimes/linux-x64/native/

# Build .NET project
cd IronBase.NET
dotnet build

# Run tests
dotnet test
```

### Native Library Caching Issue

When rebuilding the native library, .NET may cache the old version. Solution:

```bash
# Copy directly to bin folder
cp target/release/libironbase_ffi.so IronBase.NET/Demo/bin/Debug/net8.0/
```

## Testing

```bash
cd IronBase.NET

# Run all tests
dotnet test

# Run with verbosity
dotnet test -v detailed

# Run specific test
dotnet test --filter "FullyQualifiedName~TestName"
```

## Platform Support

| Platform | Architecture | Status |
|----------|--------------|--------|
| Windows | x64 | Supported |
| Linux | x64 | Supported |
| macOS | x64 | Coming soon |
| macOS | ARM64 | Coming soon |

## License

MIT License - see [LICENSE](../LICENSE) for details.

## See Also

- [Main IronBase Documentation](../README.md)
- [Indexing Guide](../INDEXES.md)
- [Aggregation Guide](../AGGREGATION.md)
