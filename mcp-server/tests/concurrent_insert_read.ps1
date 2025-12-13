# Concurrent Insert/Read Test for MCP Server
# Tests that one process can insert while another reads in parallel

param(
    [string]$McpUrl = "http://127.0.0.1:8080/mcp",
    [int]$InsertCount = 100,
    [int]$ReadCount = 200
)

$Collection = "concurrent_test_$(Get-Date -Format 'yyyyMMddHHmmss')"

Write-Host "=== Concurrent Insert/Read Test ===" -ForegroundColor Cyan
Write-Host "MCP URL: $McpUrl"
Write-Host "Collection: $Collection"
Write-Host "Inserts: $InsertCount, Reads: $ReadCount"
Write-Host ""

# Inserter job script block
$InserterScript = {
    param($Url, $Collection, $Count)

    $success = 0
    $fail = 0

    for ($i = 1; $i -le $Count; $i++) {
        $body = @{
            jsonrpc = "2.0"
            id = $i
            method = "tools/call"
            params = @{
                name = "insert_one"
                arguments = @{
                    collection = $Collection
                    document = @{
                        seq = $i
                        timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                        data = "test_record_$i"
                    }
                }
            }
        } | ConvertTo-Json -Depth 10

        try {
            $result = Invoke-RestMethod -Uri $Url -Method Post -Body $body -ContentType "application/json" -ErrorAction Stop
            if ($result.result) { $success++ } else { $fail++ }
        } catch {
            $fail++
        }

        Start-Sleep -Milliseconds 20
    }

    return @{ Success = $success; Fail = $fail }
}

# Reader job script block
$ReaderScript = {
    param($Url, $Collection, $Count)

    $success = 0
    $fail = 0
    $maxDocs = 0

    for ($i = 1; $i -le $Count; $i++) {
        $body = @{
            jsonrpc = "2.0"
            id = 1000 + $i
            method = "tools/call"
            params = @{
                name = "find"
                arguments = @{
                    collection = $Collection
                    query = @{}
                }
            }
        } | ConvertTo-Json -Depth 10

        try {
            $result = Invoke-RestMethod -Uri $Url -Method Post -Body $body -ContentType "application/json" -ErrorAction Stop
            if ($result.result) {
                $success++
                # Parse result to count documents
                $content = $result.result.content | Where-Object { $_.type -eq "text" } | Select-Object -First 1
                if ($content) {
                    $docs = ($content.text | Select-String -Pattern '"seq"' -AllMatches).Matches.Count
                    if ($docs -gt $maxDocs) { $maxDocs = $docs }
                }
            } else {
                $fail++
            }
        } catch {
            $fail++
        }

        Start-Sleep -Milliseconds 10
    }

    return @{ Success = $success; Fail = $fail; MaxDocs = $maxDocs }
}

Write-Host "=== Starting concurrent test ===" -ForegroundColor Yellow
Write-Host "Inserter and Reader running in parallel..."
Write-Host ""

# Start both jobs in parallel
$inserterJob = Start-Job -ScriptBlock $InserterScript -ArgumentList $McpUrl, $Collection, $InsertCount
$readerJob = Start-Job -ScriptBlock $ReaderScript -ArgumentList $McpUrl, $Collection, $ReadCount

# Wait for both jobs
$inserterResult = Receive-Job -Job $inserterJob -Wait
$readerResult = Receive-Job -Job $readerJob -Wait

# Clean up jobs
Remove-Job -Job $inserterJob, $readerJob

Write-Host "INSERTER: $($inserterResult.Success) success, $($inserterResult.Fail) fail" -ForegroundColor Green
Write-Host "READER: $($readerResult.Success) success, $($readerResult.Fail) fail, max docs seen: $($readerResult.MaxDocs)" -ForegroundColor Green

Write-Host ""
Write-Host "=== Final verification ===" -ForegroundColor Yellow

# Final count
$finalBody = @{
    jsonrpc = "2.0"
    id = 9999
    method = "tools/call"
    params = @{
        name = "find"
        arguments = @{
            collection = $Collection
            query = @{}
        }
    }
} | ConvertTo-Json -Depth 10

$finalResult = Invoke-RestMethod -Uri $McpUrl -Method Post -Body $finalBody -ContentType "application/json"
$finalContent = $finalResult.result.content | Where-Object { $_.type -eq "text" } | Select-Object -First 1
$finalCount = ($finalContent.text | Select-String -Pattern '"seq"' -AllMatches).Matches.Count

Write-Host "Final document count: $finalCount (expected: $InsertCount)"

# Cleanup - drop collection
$dropBody = @{
    jsonrpc = "2.0"
    id = 9998
    method = "tools/call"
    params = @{
        name = "collection_drop"
        arguments = @{
            name = $Collection
        }
    }
} | ConvertTo-Json -Depth 10

try {
    Invoke-RestMethod -Uri $McpUrl -Method Post -Body $dropBody -ContentType "application/json" | Out-Null
    Write-Host "Collection dropped" -ForegroundColor Gray
} catch {}

Write-Host ""
if ($finalCount -eq $InsertCount) {
    Write-Host "=== TEST PASSED ===" -ForegroundColor Green
    exit 0
} else {
    Write-Host "=== TEST FAILED ===" -ForegroundColor Red
    Write-Host "Expected $InsertCount documents, got $finalCount"
    exit 1
}
