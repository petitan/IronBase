// Full-text search example for Rhai scripts

// Ensure index exists (language: english)
let idx = db_create_fulltext_index("articles", "content", "english");
print("index: " + idx);

// Simple search with limit
let results = db_fulltext_search("articles", "content", "database", #{limit: 10});
print("found: " + results.len());

// Search with options
let opts = #{
    limit: 5,
    min_score: 0.2,
    projection: #{title: 1, content: 1}
};
let top = db_fulltext_search("articles", "content", "database", opts);
print("top: " + top.len());
