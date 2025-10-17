
chcp 65001 | Out-Null
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$PSStyle.OutputRendering = 'PlainText'

# cargo build --target x86_64-pc-windows-msvc --release
$env:DEVIT_NOTIFY_HOOK = 'Z:\scripts\devit_notify_example.ps1'

.\scripts\run_devitd_windows.ps1 -Secret 0143c321920e55bd9b17bb0d5ac8543c6fa0200961803c3ff01598e4e6f4007b -Config .\win_devit.core.toml
