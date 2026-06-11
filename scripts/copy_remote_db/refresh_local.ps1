$cs = "Server=127.0.0.1,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$conn = New-Object System.Data.SqlClient.SqlConnection $cs
$conn.Open()
$cmd = $conn.CreateCommand()
$cmd.CommandText = "SELECT s.name + '.' + t.name AS tblname, ISNULL(p.rows, 0) FROM sys.tables t INNER JOIN sys.schemas s ON t.schema_id = s.schema_id LEFT JOIN sys.partitions p ON t.object_id = p.object_id AND p.index_id IN (0,1) WHERE t.is_ms_shipped = 0 AND t.type = 'U' ORDER BY s.name, t.name"
$rdr = $cmd.ExecuteReader()
$sb = New-Object System.Text.StringBuilder
while ($rdr.Read()) {
    [void]$sb.AppendLine(("{0,-40} {1}" -f $rdr[0], $rdr[1]))
}
$rdr.Close()
$conn.Close()
[System.IO.File]::WriteAllText('C:\Users\Administrator\Desktop\ERP\server-rust\scripts\copy_remote_db\local_after.txt', $sb.ToString(), [System.Text.Encoding]::UTF8)
Write-Host "local_after.txt updated"
