$log = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data.log'
$errLines = $log | Where-Object { $_ -match '^ERR' }
$emptyLines = $log | Where-Object { $_ -match '^EMPTY' }
$okLines = $log | Where-Object { $_ -match '^OK' }
Write-Host ("Total OK: {0}, ERR: {1}, EMPTY: {2}" -f $okLines.Count, $errLines.Count, $emptyLines.Count)
Write-Host ""
Write-Host "=== ERR tables ==="
$errLines | ForEach-Object { Write-Host $_ }
