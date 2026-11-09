param(
    [Parameter(Mandatory = $true)]
    [string] $Executable
)

$ErrorActionPreference = "Stop"
$source = (Resolve-Path -LiteralPath $Executable).Path
$root = Join-Path ([System.IO.Path]::GetTempPath()) ("nikki-smoke-" + [guid]::NewGuid().ToString("N"))
$install = Join-Path $root "install"
$working = Join-Path $root "elsewhere"
$data = Join-Path $install "DATA"
$process = $null

try {
    New-Item -ItemType Directory -Path $data, $working | Out-Null
    $target = Join-Path $install "nikki2000-converter.exe"
    Copy-Item -LiteralPath $source -Destination $target
    [byte[]] $text = [System.Text.Encoding]::ASCII.GetBytes("synthetic smoke fixture") + @(0)
    [byte[]] $body = @(8, 0, 0) + [BitConverter]::GetBytes([uint16]$text.Length) + $text
    [byte[]] $fixture = @(78, 50, 48, 1) + [BitConverter]::GetBytes([uint32]$body.Length)
    $fixture += [byte[]] ($body | ForEach-Object { $_ -bxor 0x12 })
    $month = Join-Path $data "202611.DAY"
    [System.IO.File]::WriteAllBytes($month, $fixture)
    $originalHash = (Get-FileHash -LiteralPath $month).Hash

    $process = Start-Process -FilePath $target -WorkingDirectory $working -PassThru
    $deadline = (Get-Date).AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 500
        $process.Refresh()
        if ($process.HasExited) {
            throw "Portable application exited during startup (code $($process.ExitCode))."
        }
        if ($process.MainWindowHandle -ne 0 -and $process.Responding) {
            break
        }
    } while ((Get-Date) -lt $deadline)

    if ($process.MainWindowHandle -eq 0 -or -not $process.Responding) {
        throw "Portable application did not create a responsive window."
    }
    if ((Get-FileHash -LiteralPath $month).Hash -ne $originalHash) {
        throw "Application modified the synthetic diary during startup."
    }
    Write-Output "Portable application opened a responsive window from a different working directory."
}
finally {
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -ErrorAction Stop
        $process.WaitForExit()
    }
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
