$log = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data.log'
$errs = $log | Where-Object { $_ -match '^ERR' }
Write-Host "All ERR lines:"
$errs | ForEach-Object { Write-Host $_ }

Write-Host ""
Write-Host "First 5 OK lines:"
$log | Where-Object { $_ -match '^OK' } | Select-Object -First 5 | ForEach-Object { Write-Host $_ }
