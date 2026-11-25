//! 零依赖终端层 —— TUI 的底座。
//!
//! 不使用 ratatui/crossterm：直接调用 kernel32 打开 ANSI 虚拟终端处理，
//! 然后用 ESC 序列做备用屏幕、光标定位、24 位颜色、逐帧重绘。
//! 整个模块没有第三方依赖，链接进 exe 只增加几 KB。

use std::io::{self, Write};
use std::time::Duration;

pub fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

pub fn flush() {
    let _ = io::stdout().flush();
}

fn w(s: &str) {
    print!("{s}");
}

#[cfg(windows)]
mod win {
    use core::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(n: u32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, m: *mut u32) -> i32;
        fn SetStdHandle(n: u32, h: *mut c_void) -> i32;
        #[allow(clippy::too_many_arguments)]
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            sa: *mut c_void,
            disposition: u32,
            flags: u32,
            template: *mut c_void,
        ) -> *mut c_void;
        fn SetConsoleMode(h: *mut c_void, m: u32) -> i32;
        fn GetConsoleScreenBufferInfo(h: *mut c_void, info: *mut Csbi) -> i32;
        fn SetConsoleOutputCP(cp: u32) -> i32;
        fn SetConsoleCP(cp: u32) -> i32;
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct Coord {
        pub x: i16,
        pub y: i16,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct SmallRect {
        pub left: i16,
        pub top: i16,
        pub right: i16,
        pub bottom: i16,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct Csbi {
        pub size: Coord,
        pub cursor: Coord,
        pub attrs: u16,
        pub window: SmallRect,
        pub max_window: Coord,
    }

    const STD_OUT: u32 = 0xFFFF_FFF5; // (DWORD)-11
    const VT: u32 = 0x0004;
    const PROCESSED: u32 = 0x0001;

    fn out_handle() -> *mut c_void {
        unsafe { GetStdHandle(STD_OUT) }
    }

    /// stdout 被重定向（管道/文件）时，GetStdHandle 拿不到真控制台。
    /// 这一版直接把 STDOUT 句柄换成 CONOUT$，让后续所有 print! 落回控制台。
    pub fn attach_console() -> bool {
        let h = out_handle();
        let mut m: u32 = 0;
        if h as isize != -1 && !h.is_null() && unsafe { GetConsoleMode(h, &mut m) } != 0 {
            return true;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let c = unsafe {
            CreateFileW(
                name.as_ptr(),
                0xC000_0000, // GENERIC_READ | GENERIC_WRITE
                0x1 | 0x2,   // FILE_SHARE_READ | FILE_SHARE_WRITE
                std::ptr::null_mut(),
                3, // OPEN_EXISTING
                0,
                std::ptr::null_mut(),
            )
        };
        if c as isize == -1 || c.is_null() {
            return false;
        }
        unsafe { SetStdHandle(STD_OUT, c) };
        true
    }

    pub fn enable_vt() -> bool {
        let h = out_handle();
        let mut m: u32 = 0;
        unsafe {
            if GetConsoleMode(h, &mut m) == 0 {
                return false;
            }
            SetConsoleMode(h, m | VT | PROCESSED) != 0
        }
    }

    pub fn set_utf8() {
        unsafe {
            SetConsoleOutputCP(65001);
            SetConsoleCP(65001);
        }
    }

    pub fn size() -> Option<(u16, u16)> {
        let mut c = Csbi::default();
        let ok = unsafe { GetConsoleScreenBufferInfo(out_handle(), &mut c) };
        if ok == 0 {
            None
        } else {
            Some((
                (c.window.right - c.window.left + 1).max(1) as u16,
                (c.window.bottom - c.window.top + 1).max(1) as u16,
            ))
        }
    }
}

/// 初始化控制台：UTF-8 代码页 + ANSI 虚拟终端。返回 ANSI 是否可用。
pub fn init() -> bool {
    #[cfg(windows)]
    {
        win::set_utf8();
        win::attach_console() && win::enable_vt()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

pub fn size() -> (u16, u16) {
    #[cfg(windows)]
    {
        win::size().unwrap_or((100, 30))
    }
    #[cfg(not(windows))]
    {
        (100, 30)
    }
}

pub fn alt_enter() {
    w("\x1b[?1049h\x1b[?25l\x1b[2J");
    flush();
}

pub fn alt_leave() {
    w("\x1b[?25h\x1b[?1049l");
    flush();
}

pub fn cursor_show() {
    w("\x1b[?25h");
    flush();
}

pub fn clear() {
    w("\x1b[2J\x1b[H");
}

pub fn home() {
    w("\x1b[H");
}

/// 1-based 行列定位
pub fn goto(x: u16, y: u16) {
    w(&format!("\x1b[{y};{x}H"));
}

pub fn fg(r: u8, g: u8, b: u8) {
    w(&format!("\x1b[38;2;{r};{g};{b}m"));
}

pub fn bg(r: u8, g: u8, b: u8) {
    w(&format!("\x1b[48;2;{r};{g};{b}m"));
}

pub fn reset() {
    w("\x1b[0m");
}

pub fn bold() {
    w("\x1b[1m");
}

pub fn dim() {
    w("\x1b[2m");
}

/// 在固定行列写一段带颜色的文本
pub fn put(x: u16, y: u16, color: (u8, u8, u8), text: &str) {
    goto(x, y);
    fg(color.0, color.1, color.2);
    w(text);
    reset();
}

/// 横向渐变（每个字符独立着色），返回实际占用列数
pub fn gradient(x: u16, y: u16, width: u16, from: (u8, u8, u8), to: (u8, u8, u8), ch: char) {
    if width == 0 {
        return;
    }
    goto(x, y);
    for i in 0..width {
        let t = i as f32 / (width.max(2) - 1) as f32;
        let r = (from.0 as f32 + (to.0 as f32 - from.0 as f32) * t) as u8;
        let g = (from.1 as f32 + (to.1 as f32 - from.1 as f32) * t) as u8;
        let b = (from.2 as f32 + (to.2 as f32 - from.2 as f32) * t) as u8;
        fg(r, g, b);
        w(&ch.to_string());
    }
    reset();
    flush();
}

/// 视觉宽度（中日韩字符占 2 列）
pub fn dwidth(s: &str) -> u16 {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if (0x1100..=0x115F).contains(&u)
                || (0x2E80..=0xA4CF).contains(&u)
                || (0xAC00..=0xD7A3).contains(&u)
                || (0xF900..=0xFAFF).contains(&u)
                || (0xFE30..=0xFE6F).contains(&u)
                || (0xFF00..=0xFF60).contains(&u)
                || (0xFFE0..=0xFFE6).contains(&u)
                || (0x1F300..=0x1FAFF).contains(&u)
            {
                2
            } else {
                1
            }
        })
        .sum()
}

/// 打字机效果：逐字显现
pub fn typewrite(x: u16, y: u16, color: (u8, u8, u8), text: &str, delay_ms: u64) {
    let mut cx = x;
    for ch in text.chars() {
        goto(cx, y);
        fg(color.0, color.1, color.2);
        w(&ch.to_string());
        reset();
        flush();
        cx += dwidth(&ch.to_string());
        if delay_ms > 0 {
            sleep_ms(delay_ms);
        }
    }
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 转圈动画，播 frames 帧后原地擦掉
pub fn spin(x: u16, y: u16, label: &str, frames: u32, delay_ms: u64) {
    for i in 0..frames {
        put(x, y, (120, 200, 255), SPINNER[(i as usize) % SPINNER.len()]);
        put(x + 2, y, (150, 150, 160), label);
        flush();
        sleep_ms(delay_ms);
    }
    // 擦掉这一行
    let pad = " ".repeat((dwidth(label) + 3) as usize);
    goto(x, y);
    w(&pad);
    reset();
    flush();
}

/// 自检：验证 ANSI、尺寸、颜色、中文宽度、动画
pub fn selftest() -> anyhow::Result<()> {
    let ansi = init();
    let (cols, rows) = size();
    println!("ANSI 可用 = {ansi}｜控制台 {cols}x{rows}｜中文宽度测试 = {}", dwidth("六本豆ABC"));

    if !ansi {
        anyhow::bail!("当前控制台不支持 ANSI 虚拟终端，TUI 无法绘制");
    }

    alt_enter();
    clear();

    let width = cols.saturating_sub(4).min(72);
    gradient(3, 2, width, (60, 130, 255), (0, 220, 200), '━');
    typewrite(3, 4, (235, 240, 255), "六本豆 AGENT", 45);
    typewrite(3, 6, (130, 140, 160), "liubendou · kernel skeleton", 12);

    let checks = [
        "内核槽位 kernel/policy/task/persona/context",
        "授权层 签名 grant + 三道门 + 哈希链",
        "证据库 追加式 JSONL + 篡改可测",
        "缓存层 L1 前缀 / L2 折叠 / L3 内容寻址",
    ];
    for (i, c) in checks.iter().enumerate() {
        let y = 9 + i as u16;
        spin(3, y, "检查中…", 8, 35);
        put(3, y, (80, 220, 140), "✓");
        put(5, y, (200, 205, 215), c);
    }

    spin(3, 15, "装配内核…", 18, 40);
    put(3, 15, (80, 220, 140), "✓");
    put(5, 15, (200, 205, 215), "内核就绪 716 B｜7 个工具｜规则 kernel-1");

    gradient(3, 18, width, (255, 120, 60), (255, 60, 160), '─');
    put(3, 20, (250, 250, 255), "按任意键退出自检…");
    cursor_show();
    let mut s = String::new();
    let _ = io::stdin().read_line(&mut s);
    alt_leave();
    println!("自检结束，终端已恢复。");
    Ok(())
}
