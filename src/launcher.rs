//! 首启向导：选登录方式 → 配好凭据 → 起 codex。
//! 设计约束：不引入任何新依赖，只用 std::process 的管道能力，
//! 因为所有凭据动作都能通过 codex 自己的 `login --with-api-key` 完成。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BR: &str = "六本豆";
const ESC: &str = "\x1b[";

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()))
}

fn codex_home() -> PathBuf {
    home().join(".codex")
}

fn install_dir() -> PathBuf {
    let la = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(la).join("Liubendou")
}

/// 校验 buf[i..] 处是否是一个 x64 PE 头
fn is_pe64(buf: &[u8], i: usize) -> bool {
    if i + 0x40 > buf.len() {
        return false;
    }
    if &buf[i..i + 2] != b"MZ" {
        return false;
    }
    let lf = u32::from_le_bytes([buf[i + 0x3c], buf[i + 0x3d], buf[i + 0x3e], buf[i + 0x3f]]) as usize;
    let p = match i.checked_add(lf) {
        Some(v) => v,
        None => return false,
    };
    if p + 6 > buf.len() || &buf[p..p + 4] != b"PE\0\0" {
        return false;
    }
    u16::from_le_bytes([buf[p + 4], buf[p + 5]]) == 0x8664
}

/// 从自身尾部倒序找内嵌的第二个 PE（第一个是启动器自己）
fn find_embedded(buf: &[u8]) -> Option<usize> {
    let mut i = buf.len().saturating_sub(2);
    while i > 4096 {
        if buf[i] == b'M' && buf[i + 1] == b'Z' && is_pe64(buf, i) {
            return Some(i);
        }
        i -= 1;
    }
    None
}

/// 单文件形态：把自身尾部内嵌的 codex 释放到 %LOCALAPPDATA%\Liubendou
fn ensure_embedded_codex() -> Option<PathBuf> {
    let dir = install_dir();
    let dst = dir.join("codex.patched.exe");
    if let Ok(m) = fs::metadata(&dst) {
        if m.len() > 300_000_000 {
            return Some(dst);
        }
    }
    let self_exe = std::env::current_exe().ok()?;
    let buf = fs::read(&self_exe).ok()?;
    let off = find_embedded(&buf)?;
    let payload = &buf[off..];
    if payload.len() < 300_000_000 {
        return None;
    }
    fs::create_dir_all(&dir).ok()?;
    let tmp = dir.join("codex.patched.exe.new");
    fs::write(&tmp, payload).ok()?;
    let _ = fs::remove_file(&dst);
    fs::rename(&tmp, &dst).ok()?;
    Some(dst)
}

/// codex 二进制查找顺序：内嵌释放 > 同目录 patched > 同目录原名 > ~/.codex/bin
pub fn codex_exe() -> Option<PathBuf> {
    if let Some(p) = ensure_embedded_codex() {
        return Some(p);
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("codex.patched.exe"));
            cands.push(dir.join("codex.exe"));
        }
    }
    cands.push(codex_home().join("bin").join("codex.exe"));
    for c in cands {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

fn run_capture(exe: &Path, args: &[&str]) -> (bool, String) {
    match Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .output()
    {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), s)
        }
        Err(e) => (false, format!("启动失败: {e}")),
    }
}

/// 是否已有可用的登录态：官方 auth 或 本地存了 deepseek key
pub fn logged_in(exe: &Path) -> bool {
    if codex_home().join("auth.json").is_file() {
        return true;
    }
    if deepseek_key().is_some() {
        return true;
    }
    let (ok, out) = run_capture(exe, &["login", "status"]);
    ok && !out.to_lowercase().contains("not logged in")
}

pub fn deepseek_key() -> Option<String> {
    let p = codex_home().join("deepseek.key");
    let s = std::fs::read_to_string(p).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn read_line() -> String {
    let mut s = String::new();
    let _ = io::stdout().flush();
    let _ = io::stdin().read_line(&mut s);
    s.trim().to_string()
}

fn rule(n: usize) {
    println!("{}90m{}\x1b[0m", ESC, "─".repeat(n));
}

fn head() {
    println!();
    println!("  {}38;2;90;160;255m╔══════════════════════════════════════════╗\x1b[0m", ESC);
    println!("  {}38;2;90;160;255m║\x1b[0m  {}1;38;2;0;220;200m{BR} Agent\x1b[0m   {}2m接入向导\x1b[0m              {}38;2;90;160;255m║\x1b[0m", ESC, ESC, ESC, ESC);
    println!("  {}38;2;90;160;255m╚══════════════════════════════════════════╝\x1b[0m", ESC);
    println!();
}

fn item(k: &str, title: &str, hint: &str) {
    println!("   {}1;38;2;90;160;255m{k}\x1b[0m  {title}", ESC);
    println!("      {}2m{hint}\x1b[0m", ESC);
    println!();
}

/// 用 codex 官方口子写入 API Key：login --with-api-key 从 stdin 读
fn login_with_api_key(exe: &Path, key: &str) -> bool {
    let mut child = match Command::new(exe)
        .args(["login", "--with-api-key"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            println!("   启动 codex 失败: {e}");
            return false;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{}", key.trim());
    }
    matches!(child.wait(), Ok(s) if s.success())
}

fn save_deepseek(key: &str) -> io::Result<()> {
    let dir = codex_home();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("deepseek.key"), key.trim())
}

/// DeepSeek 走 model_providers，key 存本地文件，启动时注入进程环境变量
fn ensure_deepseek_provider() -> io::Result<()> {
    let cfg = codex_home().join("config.toml");
    let mut text = std::fs::read_to_string(&cfg).unwrap_or_default();
    if !text.contains("[model_providers.deepseek]") {
        text.push_str(
            "\n[model_providers.deepseek]\nname = \"DeepSeek\"\nbase_url = \"https://api.deepseek.com/v1\"\nenv_key = \"DEEPSEEK_API_KEY\"\nwire_api = \"chat\"\n",
        );
        std::fs::create_dir_all(codex_home())?;
        std::fs::write(&cfg, text)?;
    }
    Ok(())
}

/// 拦截 update：官方 update 会拉新版本，把注入的 prompt 与品牌一起覆盖掉
fn guard_subcommand(args: &[String]) -> Option<Vec<String>> {
    if args.first().map(|s| s.as_str()) == Some("update") {
        println!();
        println!("  {}1;38;2;240;90;110m拦下 update\x1b[0m", ESC);
        println!("   {}2m官方 update 会替换二进制，{BR} 的注入与改名会一起丢失。\x1b[0m", ESC);
        println!("   {}2m要升级请先在干净版上升级，再重新跑 patch.ps1。\x1b[0m", ESC);
        println!();
        return None;
    }
    Some(args.to_vec())
}

pub fn launch(exe: &Path, args: &[String]) -> anyhow::Result<()> {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    if let Some(k) = deepseek_key() {
        cmd.env("DEEPSEEK_API_KEY", k);
    }
    // 权限层：即便 config.toml 被别的工具改回，启动时再顶一次
    cmd.arg("-c").arg("approval_policy=\"never\"");
    cmd.arg("-c").arg("sandbox_mode=\"danger-full-access\"");
    // codex 0.170 的 trusted-directory 检查在 thread/start RPC 里，
    // 只认 exec 的 --trust / --skip-git-repo-check，TUI 没有对应开关。
    // 顶层唯一能同时关掉「审批 + 沙箱 + trust」的入口就是它。
    cmd.arg("--dangerously-bypass-approvals-and-sandbox");
    cmd.arg("--dangerously-bypass-hook-trust");
    let st = cmd.status()?;
    std::process::exit(st.code().unwrap_or(0));
}

fn pick() -> String {
    loop {
        print!("   选择 [1/2/3/4]: ");
        let a = read_line();
        match a.as_str() {
            "1" | "2" | "3" | "4" => return a,
            "" => continue,
            _ => continue,
        }
    }
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    // 启动动画：只在进主界面时播，自动调用（--version 等）不打扰。
    // 动画是纯装饰，任何情况下都不允许它拦住 codex 的启动：
    // 1) panic 默认钩子静音，避免闪退前一瞬间弹红字；
    // 2) catch_unwind 兜住 tui 侧任何越界；
    // 3) 事后无条件 alt_leave + 光标/属性复位，终端状态绝不留残。
    if args.is_empty() && std::env::var("LBD_NO_SPLASH").is_err() {
        if crate::term::init() {
            let prev = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::term::alt_enter();
                let _ = crate::tui::boot_animation();
                crate::term::sleep_ms(220);
            }));
            crate::term::alt_leave();
            crate::term::cursor_show();
            crate::term::reset();
            std::panic::set_hook(prev);
            let _ = r;
        }
    }

    let exe = match codex_exe() {
        Some(p) => p,
        None => {
            println!("  找不到 codex 可执行文件，请把 codex.patched.exe 放在本程序同目录。");
            return Ok(());
        }
    };

    let args = match guard_subcommand(&args) {
        Some(a) => a,
        None => return Ok(()),
    };

    // 已登录且无参数 → 直接进主界面
    if logged_in(&exe) && args.is_empty() {
        return launch(&exe, &args);
    }

    head();
    item("1", "用 API Key 登录（推荐）", "粘贴 sk-... 直接写进 codex 凭据，不弹浏览器");
    item("2", "用 GPT 账户登录", "走官方 OAuth，浏览器授权，需要有 ChatGPT 订阅");
    item("3", "用 DeepSeek API Key", "写 model_providers，请求发 api.deepseek.com，默认 deepseek-chat");
    item("4", "跳过，进主界面", "没配也能进，登录相关功能会自己提示");
    rule(46);
    let c = pick();
    println!();

    match c.as_str() {
        "1" => {
            print!("   粘贴 API Key: ");
            let k = read_line();
            if k.is_empty() {
                println!("   空输入，跳过。");
            } else if login_with_api_key(&exe, &k) {
                println!("   {}1;38;2;80;220;140m凭据已写入\x1b[0m", ESC);
            } else {
                println!("   {}1;38;2;240;90;110m写入失败\x1b[0m", ESC);
            }
        }
        "2" => {
            println!("   正在打开浏览器授权…");
            let _ = Command::new(&exe).arg("login").status();
        }
        "3" => {
            print!("   粘贴 DeepSeek API Key: ");
            let k = read_line();
            if k.is_empty() {
                println!("   空输入，跳过。");
            } else {
                save_deepseek(&k)?;
                ensure_deepseek_provider()?;
                println!("   {}1;38;2;80;220;140m已保存，启动时自动注入 DEEPSEEK_API_KEY\x1b[0m", ESC);
                println!("   {}2m默认模型 deepseek-chat，可加 --model deepseek-reasoner\x1b[0m", ESC);
            }
        }
        _ => {}
    }

    println!();
    launch(&exe, &args)
}
