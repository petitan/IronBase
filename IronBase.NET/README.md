# IronBase

IronBase is a lightweight embedded NoSQL document database with a MongoDB-like API, powered by Rust.

## Features

- **MongoDB-like API**: Familiar `IronBaseClient`, `IronBaseCollection<T>`, and `Builders<T>` pattern
- **Embedded database**: Single-file, serverless, zero-configuration
- **Cross-platform**: Windows x64 and Linux x64 support
- **High performance**: Rust core with B+ tree indexing
- **ACID-lite transactions**: Atomicity, Consistency, Durability (no isolation)

## Quick Start

```csharp
using IronBase;

// Open database
using var client = new IronBaseClient("mydata.mlite");

// Get collection
var users = client.GetCollection<User>("users");

// Insert
users.InsertOne(new User { Name = "Alice", Age = 30 });

// Query
var filter = Builders<User>.Filter.Eq("Name", "Alice");
var user = users.FindOne(filter);

// Update
var update = Builders<User>.Update.Set("Age", 31);
users.UpdateOne(filter, update);

// Delete
users.DeleteOne(filter);
```

## Supported Operations

- **CRUD**: `InsertOne`, `InsertMany`, `Find`, `FindOne`, `UpdateOne`, `UpdateMany`, `DeleteOne`, `DeleteMany`
- **Query Operators**: `$eq`, `$ne`, `$gt`, `$gte`, `$lt`, `$lte`, `$in`, `$nin`, `$and`, `$or`, `$not`, `$exists`, `$regex`
- **Update Operators**: `$set`, `$inc`, `$unset`, `$push`, `$pull`, `$addToSet`, `$pop`
- **Indexing**: `CreateIndex`, `CreateCompoundIndex`, `DropIndex`, `ListIndexes`
- **Aggregation**: `Aggregate` with `$match`, `$group`, `$project`, `$sort`, `$limit`, `$skip`

## License

MIT
