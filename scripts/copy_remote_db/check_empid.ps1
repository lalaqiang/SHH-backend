$lines = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\failed_tables\dbo_tBas_Emp.csv' -Encoding UTF8 -TotalCount 5
foreach ($l in $lines) {
    $parts = $l -split "`t"
    Write-Host ("[0]: '{0}'" -f $parts[0])
}
