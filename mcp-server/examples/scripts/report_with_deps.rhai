// Sales Report with Dependencies
// Dependencies: helper_utils
// This script uses functions from helper_utils

// Fetch all completed orders
let orders = db_find("orders", #{ status: "completed" });
print(`Processing ${orders.len()} completed orders...`);

// Calculate totals using helper functions
let amounts = [];
for order in orders {
    amounts.push(order.amount);
}

let gross_total = sum_array(amounts);
let avg_order = average(amounts);
let max_order = find_max(amounts);
let min_order = find_min(amounts);

// Calculate tax
let tax_rate = 0.08;
let tax_amount = calculate_tax(gross_total, tax_rate);
let net_total = calculate_with_tax(gross_total, tax_rate);

// Build report
#{
    report_title: "Sales Summary Report",
    period: "2024-01",
    metrics: #{
        total_orders: orders.len(),
        gross_total: format_currency(gross_total),
        tax: format_currency(tax_amount),
        net_total: format_currency(net_total),
        average_order: format_currency(avg_order),
        largest_order: format_currency(max_order),
        smallest_order: format_currency(min_order)
    },
    tax_rate: format_percentage(tax_rate)
}
