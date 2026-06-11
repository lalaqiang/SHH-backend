$localTables = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\local_tables.txt' | ForEach-Object { $_.Trim() } | Where-Object { $_ }
$remoteTables = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\remote_tables.txt' | ForEach-Object { $_.Trim() } | Where-Object { $_ }

Write-Host "Local tables: $($localTables.Count), Remote tables: $($remoteTables.Count)"
Write-Host ""
Write-Host "Tables in REMOTE but NOT in LOCAL (will be created):"
Compare-Object -ReferenceObject $localTables -DifferenceObject $remoteTables | Where-Object { $_.SideIndicator -eq '=>' } | ForEach-Object { Write-Host "  $($_.InputObject)" }
Write-Host ""
Write-Host "Tables in LOCAL but NOT in REMOTE (will be skipped):"
Compare-Object -ReferenceObject $localTables -DifferenceObject $remoteTables | Where-Object { $_.SideIndicator -eq '<=' } | ForEach-Object { Write-Host "  $($_.InputObject)" }
