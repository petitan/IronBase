// Data Aggregation Example
// Demonstrates the db_aggregate function with MongoDB-style pipeline

// Aggregation: Group orders by status and calculate totals
let status_summary = db_aggregate("orders", [
    #{ "$group": #{
        "_id": "$status",
        "count": #{ "$sum": 1 },
        "total_amount": #{ "$sum": "$amount" },
        "avg_amount": #{ "$avg": "$amount" }
    }},
    #{ "$sort": #{ "total_amount": -1 }}
]);

print("Order Status Summary:");
for status in status_summary {
    print(`  ${status._id}: ${status.count} orders, total: $${status.total_amount}`);
}

// Aggregation: Top customers by order value
let top_customers = db_aggregate("orders", [
    #{ "$match": #{ "status": "completed" }},
    #{ "$group": #{
        "_id": "$customer_id",
        "order_count": #{ "$sum": 1 },
        "total_spent": #{ "$sum": "$amount" }
    }},
    #{ "$sort": #{ "total_spent": -1 }},
    #{ "$limit": 10 }
]);

print("\nTop 10 Customers:");
let rank = 1;
for customer in top_customers {
    print(`  ${rank}. Customer ${customer._id}: $${customer.total_spent} (${customer.order_count} orders)`);
    rank += 1;
}

// Aggregation: Monthly revenue
let monthly_revenue = db_aggregate("orders", [
    #{ "$match": #{ "status": "completed" }},
    #{ "$group": #{
        "_id": #{ "$substr": ["$created_at", 0, 7] },  // Extract YYYY-MM
        "revenue": #{ "$sum": "$amount" },
        "orders": #{ "$sum": 1 }
    }},
    #{ "$sort": #{ "_id": 1 }}
]);

// Return all results
#{
    status_breakdown: status_summary,
    top_customers: top_customers,
    monthly_revenue: monthly_revenue
}
