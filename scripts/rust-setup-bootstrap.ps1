# Standalone installer adapter. Dependency policy and install effects remain in the ZIP bootstrap.
param([string] $Bundle = '', [switch] $Unattended)
$ErrorActionPreference = 'Stop'

function Get-SetupExitCode($Result) {
    if ($Result.state -eq 'setup-returned') {
        if ($Result.exitCode -eq 0) { return 0 }
        return 5
    }
    switch ($Result.state) {
        'cancelled' { return 2 }
        'runtime-required' { return 3 }
        'restart-required' { return 4 }
        default { return 5 }
    }
}

function Invoke-FuzeSetup([string] $Payload, [bool] $Silent) {
    $script:SetupPhase = 'architecture'
    if (-not [Environment]::Is64BitProcess) { throw 'setup-x64-required' }
    $script:SetupPhase = 'load-bootstrap'
    . (Join-Path $Payload 'Install-local.ps1')
    $script:SetupPhase = 'contract'
    $contract = Read-VcContract $Payload
    $inspect = { @(Get-VcInventory $contract) }
    $choose = {
        param($c)
        if ($Silent) { return '' }
        Add-Type -AssemblyName System.Windows.Forms
        $answer = [Windows.Forms.MessageBox]::Show(
            "需要 Microsoft Visual C++ x64 $($c.minimumVersion) 或更新版本。`n`n是否打开微软下载地址？下载完成后，请在下一步选择 VC_redist.x64.exe。取消将停止安装 FUZE。",
            'FUZE · 运行库', 'YesNo', 'Information', 'Button2')
        if ($answer -ne 'Yes') { return '' }
        # Read-VcContract has already restricted this to the reviewed Microsoft HTTPS URL.
        Start-Process -FilePath $c.redistributable.url
        $dialog = New-Object Windows.Forms.OpenFileDialog
        try {
            $dialog.Title = '选择已下载的 Microsoft VC_redist.x64.exe'
            $dialog.Filter = 'Microsoft VC++ installer|VC_redist.x64.exe'
            if ($dialog.ShowDialog() -eq 'OK') { return $dialog.FileName }
            return ''
        } finally { $dialog.Dispose() }
    }
    $confirm = {
        [Windows.Forms.MessageBox]::Show(
            '允许安装此 Microsoft 运行库？将核对散列、微软签名和版本，再请求 Windows 管理员授权；不会自动重启。',
            'FUZE · 确认运行库安装', 'YesNo', 'Question', 'Button2') -eq 'Yes'
    }
    $validate = { param($file,$c) Assert-VcInstaller $file $c }
    $install = {
        param($file)
        try {
            $p = Start-Process -FilePath $file -ArgumentList '/install','/passive','/norestart' -Verb RunAs -WindowStyle Hidden -Wait -PassThru
            return $p.ExitCode
        } catch {
            $cause = $_.Exception
            for ($i=0; $i -lt 4 -and $cause; $i++) {
                if ($cause -is [ComponentModel.Win32Exception] -and $cause.NativeErrorCode -eq 1223) { return 1223 }
                $cause = $cause.InnerException
            }
            throw 'runtime-installer-launch-failed'
        }
    }
    $setup = {
        $script:SetupPhase = 'native-preflight'
        $start = Get-VcStartInfo $Payload
        # The wizard Install button (or explicit /VERYSILENT) supplies confirmation.
        $start.Arguments = '--install-local-confirmed'
        $script:SetupPhase = 'native-install'
        $p = [Diagnostics.Process]::Start($start)
        try { $p.WaitForExit(); return $p.ExitCode } finally { $p.Dispose() }
    }
    $script:SetupPhase = 'runtime-inventory'
    if ($Silent -and (Get-VcDecision $contract (& $inspect)).state -eq 'runtime-required') {
        return [pscustomobject]@{state='runtime-required'}
    }
    return Invoke-VcFlow $contract $inspect $choose $confirm $validate $install $setup
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        $result = Invoke-FuzeSetup $Bundle $Unattended.IsPresent
        [Console]::Error.WriteLine("fuze-setup-state-$($result.state)")
        exit (Get-SetupExitCode $result)
    } catch {
        # Never surface raw exception paths, commands, or runtime installer output.
        [Console]::Error.WriteLine("fuze-setup-failed-$script:SetupPhase")
        [Console]::Error.WriteLine("fuze-setup-error-type-$($_.Exception.GetType().Name)")
        [Console]::Error.WriteLine("fuze-setup-error-line-$($_.InvocationInfo.ScriptLineNumber)")
        if ($_.Exception.Message -match '^runtime-[a-z-]{1,64}$') {
            [Console]::Error.WriteLine("fuze-setup-reason-$($_.Exception.Message)")
        }
        exit 5
    }
}
