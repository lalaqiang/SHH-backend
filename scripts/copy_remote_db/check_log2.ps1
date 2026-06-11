$log = Get-Content 'C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\02_data.log'
$groups = @{}
foreach ($line in $log) {
    if ($line -match '^(OK|ERR|EMPTY|SKIP)\s+(\S+)') {
        $type = $Matches[1]
        if (-not $groups.ContainsKey($type)) { $groups[$type] = @() }
        $groups[$type] += $Matches[2]
    }
}
foreach ($k in $groups.Keys) {
    Write-Host "$k : $($groups[$k].Count) tables"
}
Write-Host ""
Write-Host "The 6 new tables status:"
foreach ($t in @('dbo.tPub_DocImg', 'dbo.tStk_StockHis', 'dbo.tsys_GridInfo20201109New', 'dbo.tSys_TranHis', 'dbo.tSys_User', 'dbo.表1')) {
    $status = $log | Where-Object { $_ -match "^\S+\s+$([regex]::Escape($t))\s" } | Select-Object -First 1
    Write-Host "  $t : $status"
}
Write-Host ""
Write-Host "ERR tables:"
foreach ($t in $groups['ERR']) { Write-Host "  $t" }
