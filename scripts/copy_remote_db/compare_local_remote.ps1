$local = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\local_after.txt' | ForEach-Object { $_.Trim() } | Where-Object { $_ -and $_ -notmatch '^\(\d' } | Where-Object { $_ -notmatch 'rows affected' }
$remote = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\remote_tables.txt' | ForEach-Object { $_.Trim() } | Where-Object { $_ }

# Get remote row counts
$rc = New-Object System.Data.SqlClient.SqlConnection "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$rc.Open()
$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT s.name + '.' + t.name + '|' + CAST(p.rows AS VARCHAR) FROM sys.tables t INNER JOIN sys.schemas s ON t.schema_id = s.schema_id LEFT JOIN sys.partitions p ON t.object_id = p.object_id AND p.index_id IN (0,1) WHERE t.is_ms_shipped = 0 AND t.type = 'U' ORDER BY s.name, t.name"
$rdr = $cmd.ExecuteReader()
$remoteData = @{}
while ($rdr.Read()) {
    $parts = $rdr[0] -split '\|', 2
    $remoteData[$parts[0]] = [int]$parts[1]
}
$rdr.Close()
$rc.Close()

# Parse local
$localData = @{}
foreach ($line in $local) {
    $parts = $line -split '\s+', 2
    if ($parts.Count -eq 2) {
        $name = $parts[0].Trim()
        $rows = if ([string]::IsNullOrEmpty($parts[1])) { 0 } else { [int]$parts[1] }
        $localData[$name] = $rows
    }
}

# Compare
Write-Host ("Table                          Local    Remote    Diff")
Write-Host ("----------------------------------------------------------------")
$localOnly = 0; $remoteOnly = 0; $both = 0; $diff = 0
foreach ($key in $localData.Keys) {
    $lr = $localData[$key]
    $rr = if ($remoteData.ContainsKey($key)) { $remoteData[$key] } else { -1 }
    if ($rr -eq -1) {
        Write-Host ("{0,-30}  {1,8}  {2,8}  LOCAL ONLY" -f $key, $lr, "-")
        $localOnly++
    } elseif ($lr -eq $rr) {
        $both++
    } else {
        Write-Host ("{0,-30}  {1,8}  {2,8}  diff={3}" -f $key, $lr, $rr, ($lr - $rr))
        $diff++
    }
}
foreach ($key in $remoteData.Keys) {
    if (-not $localData.ContainsKey($key)) {
        Write-Host ("{0,-30}  {1,8}  {2,8}  REMOTE ONLY" -f $key, "-", $remoteData[$key])
        $remoteOnly++
    }
}
Write-Host ""
Write-Host ("LocalOnly: {0}, RemoteOnly: {1}, Matched: {2}, Diff: {3}, Total Local: {4}, Total Remote: {5}" -f $localOnly, $remoteOnly, $both, $diff, $localData.Count, $remoteData.Count)
