$rc = New-Object System.Data.SqlClient.SqlConnection "Server=shenhuahui.f3322.org,1433;Database=TestERP;User ID=sa;Password=sa123456;Encrypt=False;TrustServerCertificate=True"
$rc.Open()
$cmd = $rc.CreateCommand()
$cmd.CommandText = "SELECT c.name, t.name, t.name as n2, LEN(t.name) as l FROM sys.columns c INNER JOIN sys.types t ON c.user_type_id = t.user_type_id WHERE c.object_id = OBJECT_ID('dbo.tBas_Emp') AND c.name='EmpID'"
$rdr = $cmd.ExecuteReader()
$rdr.Read()
$v = $rdr.GetValue(1)
Write-Host ("Type: '{0}'  Length: {1}  Bytes: {2}" -f $v, $v.ToString().Length, [System.Text.Encoding]::UTF8.GetBytes([string]$v).Length)
Write-Host ("Equals 'uniqueidentifier': {0}" -f ($v -eq 'uniqueidentifier'))
Write-Host ("Bytes: {0}" -f ([System.Text.Encoding]::GetEncoding(936).GetBytes([string]$v) -join ','))
$rdr.Close()
$rc.Close()
