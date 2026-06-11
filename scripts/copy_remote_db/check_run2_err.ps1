# Find table with GDSAbc column
$content = [System.IO.File]::ReadAllText('C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data.sql', [System.Text.Encoding]::UTF8)
$lines = $content -split "`n"
$idx = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match 'GDSAbc') {
        Write-Host "Found GDSAbc at line $i"
        $idx = $i
        break
    }
}
# Find the table marker before this line
$j = $idx
while ($j -gt 0 -and -not ($lines[$j] -match '^-- dbo\.')) { $j-- }
Write-Host "Table marker: $($lines[$j])"
# Find the GO after the table
$k = $idx
while ($k -lt $lines.Count -and -not ($lines[$k] -match '^GO$')) { $k++ }
Write-Host "Next GO at line $k"
# Show the first INSERT for this table
for ($m = $j; $m -lt ($j + 10); $m++) {
    Write-Host ("[{0}]: {1}" -f $m, $lines[$m])
}
