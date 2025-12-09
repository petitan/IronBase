// Basic Query Example
// Find all documents in a collection with optional filtering

// Simple find - get all users
let all_users = db_find("users", #{});
print(`Found ${all_users.len()} total users`);

// Find with filter - active users only
let active_users = db_find("users", #{ status: "active" });
print(`Found ${active_users.len()} active users`);

// Find with comparison operators
let adult_users = db_find("users", #{
    age: #{ "$gte": 18 }
});
print(`Found ${adult_users.len()} adult users`);

// Return the results
#{
    total: all_users.len(),
    active: active_users.len(),
    adults: adult_users.len()
}
