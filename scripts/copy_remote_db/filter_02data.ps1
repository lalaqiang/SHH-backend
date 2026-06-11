# Create modified 02_data.sql excluding the 5 tables that were loaded via load_failed.ps1
$content = [System.IO.File]::ReadAllText('C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data.sql', [System.Text.Encoding]::UTF8)
$lines = $content -split "`n"
$skip = @('dbo.tBas_Emp','dbo.tBas_EmpApply','dbo.tmp_tbas_Emp','dbo.tOA_LineMan','dbo.tSys_MD')

$sb = New-Object System.Text.StringBuilder
$skipSection = $false
for ($i = 0; $i -lt $lines.Count; $i++) {
    $line = $lines[$i]
    if ($line -match '^-- (dbo\.[\w$]+)\s+\(\d+ rows\)') {
        $tname = $matches[1]
        if ($skip -contains $tname) {
            $skipSection = $true
            continue
        } else {
            $skipSection = $false
        }
    }
    if ($skipSection) {
        if ($line -match '^GO$') {
            $skipSection = $false
            continue
        }
        continue
    }
    [void]$sb.AppendLine($line)
}
[System.IO.File]::WriteAllText('C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data_v2.sql', $sb.ToString(), [System.Text.Encoding]::UTF8)
Write-Host "Written 02_data_v2.sql"
$newSize = (Get-Item 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data_v2.sql').Length
Write-Host ("New size: {0} MB" -f [math]::Round($newSize/1MB, 2))
