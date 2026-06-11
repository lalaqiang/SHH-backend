# Find table with problematic data in 02_data_v2.sql
# Look for INSERT statements that have a string value with unmatched quotes
$content = [System.IO.File]::ReadAllText('C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data_v2.sql', [System.Text.Encoding]::UTF8)
$lines = $content -split "`n"
$curTable = ""
$curLines = @()
$problems = @()
for ($i = 0; $i -lt $lines.Count; $i++) {
    $line = $lines[$i]
    if ($line -match '^-- (dbo\.[\w$]+)\s+\(\d+ rows\)') {
        $curTable = $matches[1]
        $curLines = @()
        continue
    }
    if ($line -match '^GO$') {
        if ($curLines.Count -gt 0) {
            # Check each INSERT line for unbalanced quotes
            foreach ($cl in $curLines) {
                if ($cl -match "^INSERT INTO \[$($curTable -replace 'dbo\.','')\]") {
                    # Count quotes (excluding doubled ones)
                    $clean = $cl -replace "''", ""
                    $quoteCount = ([regex]::Matches($clean, "'")).Count
                    if (($quoteCount % 2) -ne 0) {
                        $problems += [PSCustomObject]@{
                            Table = $curTable
                            Line = $cl.Substring(0, [Math]::Min(200, $cl.Length))
                        }
                        break
                    }
                }
            }
        }
        $curTable = ""
        $curLines = @()
        continue
    }
    if ($line.StartsWith('INSERT INTO') -or $line.StartsWith('DELETE') -or $line.StartsWith('SET IDENTITY')) {
        $curLines += $line
    }
}
Write-Host "Found problems: $($problems.Count)"
$problems | Select-Object -First 20 | ForEach-Object { Write-Host ("--- $($_.Table) ---"); Write-Host $_.Line }
