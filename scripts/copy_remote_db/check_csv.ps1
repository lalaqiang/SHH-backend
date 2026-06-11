$h = (Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\failed_tables\dbo_tBas_Emp.csv' -Encoding UTF8 -TotalCount 1)
$cols = $h -split "`t"
$idx = 0
foreach ($c in $cols) {
    Write-Host ("[{0}] '{1}' len={2}" -f $idx, $c, $c.Length)
    $idx++
}
