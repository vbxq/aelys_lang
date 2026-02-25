$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
$vc = "$vs\VC\Auxiliary\Build\vcvars64.bat"
cmd /c ('"' + $vc + '" >nul 2>&1 && set') | ForEach-Object {
    if ($_ -match '^(.*?)=(.*)$') {
        Set-Item -Path "Env:$($matches[1])" -Value $matches[2]
    }
}
$env:LLVM_SYS_181_PREFIX = 'C:\llvm'
$env:PATH = "C:\llvm\bin;$env:PATH"
Set-Location 'C:\Users\admin\RustroverProjects\aelys_lang'
.\target\debug\aelys-cli.exe compile .\main.aelys --backend llvm --emit-llvm-ir