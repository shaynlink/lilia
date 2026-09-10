$ErrorActionPreference = 'Stop'
# Only the disposable hosted CI machine receives a local account. No fixed
# password, existing account mutation, or credential-bearing command arguments.
$name = 'lilia' + [Guid]::NewGuid().ToString('N').Substring(0, 12)
$password = 'Aa1!' + [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
Write-Output "::add-mask::$password"
$account = $null
try {
    $account = New-LocalUser -Name $name -Password (ConvertTo-SecureString $password -AsPlainText -Force)
    $users = Get-LocalGroup -SID 'S-1-5-32-545'
    Add-LocalGroupMember -Group $users -Member $account
    $env:LILIA_TEST_USER = $name
    $env:LILIA_TEST_PASSWORD = $password
    $test = 'ipc_windows::tests::cross_account::second_account_cannot_open_private_pipe_or_token'
    $listed = cargo test -p lilia-daemon $test -- --ignored --exact --list
    if ($LASTEXITCODE -ne 0 -or $listed -notcontains "${test}: test") {
        throw 'Expected cross-account test was not discovered'
    }
    cargo test -p lilia-daemon $test -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw 'Cross-account test failed' }
} finally {
    Remove-Item Env:LILIA_TEST_USER, Env:LILIA_TEST_PASSWORD -ErrorAction SilentlyContinue
    $password = $null
    if ($null -ne $account) { Remove-LocalUser -SID $account.SID }
}
