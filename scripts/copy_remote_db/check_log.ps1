$log = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data.log'
$groups = $log | ForEach-Object {
    if ($_ -match '^(OK|ERR|EMPTY|SKIP)\s') { $Matches[1] } else { 'OTHER' }
} | Group-Object
Write-Host "Log entry counts:"
foreach ($g in $groups) { Write-Host ("  {0}: {1}" -f $g.Name, $g.Count) }

# show first 5 ERR
$errs = $log | Where-Object { $_ -match '^ERR' } | Select-Object -First 5
Write-Host ""
Write-Host "First errors:"
foreach ($e in $errs) { Write-Host "  $e" }

# show last 5
$last5 = $log | Select-Object -Last 5
Write-Host ""
Write-Host "Last 5 log entries:"
foreach ($e in $last5) { Write-Host "  $e" }
