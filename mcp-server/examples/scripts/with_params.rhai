// Script with Parameters Example
// Demonstrates how to use the params object

// Get parameters from the caller
let collection = params.collection;
let min_value = params.min_value;
let max_value = params.max_value;
let field_name = params.field_name;

// Validate parameters
if collection == () {
    throw "Missing required parameter: collection";
}

// Build dynamic query based on params
let query = #{};
if min_value != () {
    query[field_name] = #{ "$gte": min_value };
}
if max_value != () && min_value != () {
    query[field_name] = #{ "$gte": min_value, "$lte": max_value };
}

// Execute query
let result = db_find(collection, query);

// Return results with metadata
#{
    collection: collection,
    query: query,
    result_count: result.documents.len(),
    results: result.documents
}
