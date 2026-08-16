$pwd = '123456'
$salt = 'erp_shenhuihui_2024'
$input = "$pwd$salt"
$bytes = [System.Text.Encoding]::UTF8.GetBytes($input)
$sha = [System.Security.Cryptography.SHA256]::Create()
$hashBytes = $sha.ComputeHash($bytes)
$hash = ([System.BitConverter]::ToString($hashBytes)).Replace('-','').ToLower()
Write-Output "SHA256:$hash"
