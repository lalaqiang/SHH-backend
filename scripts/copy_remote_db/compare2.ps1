$local = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\local_tables.txt' | ForEach-Object { $_.Trim() } | Where-Object { $_ }
$remote = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\remote_tables.txt' | ForEach-Object { $_.Trim() } | Where-Object { $_ }

Write-Host "Local tables count: $($local.Count)"
Write-Host "Remote tables count: $($remote.Count)"
Write-Host ""
Write-Host "In REMOTE but not in LOCAL:"
$diff1 = Compare-Object -ReferenceObject $local -DifferenceObject $remote | Where-Object { $_.SideIndicator -eq '=>' }
foreach ($d in $diff1) { Write-Host "  $($d.InputObject)" }
Write-Host ""
Write-Host "In LOCAL but not in REMOTE:"
$diff2 = Compare-Object -ReferenceObject $local -DifferenceObject $remote | Where-Object { $_.SideIndicator -eq '<=' }
foreach ($d in $diff2) { Write-Host "  $($d.InputObject)" }
