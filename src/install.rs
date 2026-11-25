//! 自安装：复制自身 → 建桌面/开始菜单快捷方式 → 注册卸载项。
//!
//! 刻意不引 Inno / NSIS：安装包每多一层壳，就多一份要维护的静默参数与提权面。
//! 全部落在 HKCU 与 %LOCALAPPDATA%，不碰 HKLM、不需要管理员。
//!
//! 中文必须走 .ps1 文件（UTF-8 BOM）而不是 -Command 内联参数：
//! powershell 5.1 从命令行收中文会被系统代码页打成问号，快捷方式名就变成 ???。

use std::path::{Path, PathBuf};
use std::process::Command;

const APP: &str = "六本豆 Agent";
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Liubendou";

fn local_appdata() -> anyhow::Result<PathBuf> {
    let p = std::env::var("LOCALAPPDATA").map_err(|_| anyhow::anyhow!("环境变量 LOCALAPPDATA 缺失"))?;
    Ok(PathBuf::from(p))
}

/// 安装根目录：%LOCALAPPDATA%\Liubendou
pub fn install_dir() -> anyhow::Result<PathBuf> {
    Ok(local_appdata()?.join("Liubendou"))
}

pub fn installed_exe() -> anyhow::Result<PathBuf> {
    Ok(install_dir()?.join("liubendou.exe"))
}

/// 把脚本写成 UTF-8 BOM 的临时 .ps1 再执行，彻底绕开命令行编码问题
fn run_ps_file(script: &str) -> anyhow::Result<String> {
    let mut path = std::env::temp_dir();
    path.push(format!("liubendou-{}.ps1", std::process::id()));
    let mut bytes = Vec::with_capacity(script.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(script.as_bytes());
    std::fs::write(&path, &bytes).map_err(|e| anyhow::anyhow!("写临时脚本失败：{e}"))?;

    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &path.display().to_string(),
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("调不起 powershell：{e}"));
    let _ = std::fs::remove_file(&path);
    let out = out?;

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("powershell 失败：{}", stderr.trim());
    }
    Ok(stdout)
}

pub fn install() -> anyhow::Result<()> {
    let src = std::env::current_exe()?;
    let dir = install_dir()?;
    std::fs::create_dir_all(&dir)?;
    let exe = dir.join("liubendou.exe");

    let same = match (src.canonicalize(), exe.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        std::fs::copy(&src, &exe).map_err(|e| anyhow::anyhow!("复制自身到 {} 失败：{e}", exe.display()))?;
        println!("✓ 已安装到 {}", exe.display());
    } else {
        println!("✓ 已是最新安装：{}", exe.display());
    }

    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$exe = '{exe}'
$dir = '{dir}'
$w = New-Object -ComObject WScript.Shell
$bases = @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))
foreach ($base in $bases) {{
  $lnk = Join-Path $base '{app}.lnk'
  $tmp = Join-Path $base ('lbd-' + [guid]::NewGuid().ToString('N').Substring(0,8) + '.lnk')
  if (Test-Path $tmp) {{ Remove-Item $tmp -Force }}
  $s = $w.CreateShortcut($tmp)
  $s.TargetPath = $exe
  $s.WorkingDirectory = $dir
  $s.Description = 'LBD'
  $s.IconLocation = $exe + ',0'
  $s.Save()
  if (Test-Path $lnk) {{ Remove-Item $lnk -Force }}
  Move-Item $tmp $lnk -Force
  Write-Output ('LNK ' + $lnk)
}}
$key = '{key}'
New-Item -Path ('Registry::' + $key) -Force | Out-Null
$pairs = [ordered]@{{
  DisplayName    = '{app}'
  DisplayVersion = '{version}'
  Publisher      = '六本豆'
  InstallLocation= $dir
  DisplayIcon    = $exe + ',0'
  UninstallString= '"' + $exe + '" uninstall'
  NoModify       = '1'
  NoRepair       = '1'
}}
foreach ($k in $pairs.Keys) {{
  New-ItemProperty -Path ('Registry::' + $key) -Name $k -Value $pairs[$k] -PropertyType String -Force | Out-Null
}}
Write-Output 'REG OK'
"#,
        exe = exe.display(),
        dir = dir.display(),
        app = APP,
        key = KEY,
        version = env!("CARGO_PKG_VERSION"),
    );

    let out = run_ps_file(&script)?;
    let mut lnk = 0;
    for line in out.lines() {
        if let Some(p) = line.trim().strip_prefix("LNK ") {
            println!("✓ 快捷方式 {p}");
            lnk += 1;
        }
    }
    if lnk == 0 {
        anyhow::bail!("一个快捷方式都没建成，安装不完整");
    }
    if out.contains("REG OK") {
        println!("✓ 已登记到「应用和功能」（HKCU，无需管理员）");
    }

    println!();
    println!("安装完成。双击桌面「{APP}」即可进入。");
    println!("数据目录：{}", dir.join("liubendou-data").display());
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let dir = install_dir()?;
    let exe = dir.join("liubendou.exe");

    let script = format!(
        r#"$ErrorActionPreference = 'Continue'
$w = New-Object -ComObject WScript.Shell
$bases = @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))
foreach ($base in $bases) {{
  $lnk = Join-Path $base '{app}.lnk'
  if (Test-Path $lnk) {{ Remove-Item $lnk -Force; Write-Output ('DEL ' + $lnk) }}
}}
Remove-Item -Path ('Registry::' + '{key}') -Recurse -Force -ErrorAction SilentlyContinue
Write-Output 'REG DEL'
"#,
        app = APP,
        key = KEY,
    );
    if let Ok(out) = run_ps_file(&script) {
        for line in out.lines() {
            if let Some(p) = line.trim().strip_prefix("DEL ") {
                println!("✓ 已删除 {p}");
            }
        }
        println!("✓ 已移除卸载登记");
    }

    // 数据目录（先问）
    let data = dir.join("liubendou-data");
    let mut keep = true;
    if data.exists() {
        print!("保留证据库与授权记录？[Y/n] ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
        keep = !matches!(s.trim(), "n" | "N" | "no" | "NO");
    }

    // 程序本体
    let me = std::env::current_exe().ok();
    if let Some(m) = &me {
        if m == &exe {
            println!("⚠ 正在运行安装目录里的副本，退出后请手动删除 {}", exe.display());
        } else if exe.exists() && std::fs::remove_file(&exe).is_ok() {
            println!("✓ 已删除 {}", exe.display());
        }
    }

    if !keep {
        let _ = std::fs::remove_dir_all(&data);
        println!("✓ 已删除数据目录");
    } else if data.exists() {
        println!("· 数据保留在 {}", data.display());
    }

    let _ = std::fs::remove_dir(&dir);
    println!("卸载完成。");
    Ok(())
}

/// 供 doctor 检查安装状态
pub fn is_installed() -> Option<PathBuf> {
    let p: PathBuf = installed_exe().ok()?;
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// 卸载入口（保持 Path 引入不报警）
#[allow(dead_code)]
fn _unused(p: &Path) -> bool {
    p.exists()
}
