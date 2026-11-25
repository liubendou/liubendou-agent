//! TUI 主体：启动动画 + 主界面。
//!
//! 视觉结构照 codex TUI：一屏大字 + 版本行 + spinner 自检 → 状态栏 + 会话流 + 输入框。
//! 字模用 figlet 的 ANSI Shadow（L / B / D 三个字母），中文标题交给终端本身渲染。

use std::io::{self, Write};

use crate::term;
use crate::{guard, store, TOOLS_TOTAL};

/// ANSI Shadow 字模：每行必须等宽
const GLYPH_L: [&str; 6] = [
    "    ██╗     ",
    "    ██║     ",
    "    ██║     ",
    "    ██║     ",
    "    ███████╗",
    "    ╚══════╝",
];
const GLYPH_B: [&str; 6] = [
    "    ██████╗ ",
    "    ██╔══██╗",
    "    ██████╔╝",
    "    ██╔══██╗",
    "    ██████╔╝",
    "    ╚═════╝ ",
];
const GLYPH_D: [&str; 6] = [
    "    ██████╗ ",
    "    ██╔══██╗",
    "    ██║  ██║",
    "    ██║  ██║",
    "    ██████╔╝",
    "    ╚═════╝ ",
];

/// 纯 ASCII 回退字模（终端把 ambiguous 字符算成 2 列时用 --ascii）
const ASCII_L: [&str; 6] = [
    "    ##      ",
    "    ##      ",
    "    ##      ",
    "    ##      ",
    "    ########",
    "    ++++++++",
];
const ASCII_B: [&str; 6] = [
    "    #####   ",
    "    ##  ##  ",
    "    #####   ",
    "    ##  ##  ",
    "    #####   ",
    "    +++++   ",
];
const ASCII_D: [&str; 6] = [
    "    #####   ",
    "    ##  ##  ",
    "    ##  ##  ",
    "    ##  ##  ",
    "    #####   ",
    "    +++++   ",
];

const THEME_A: (u8, u8, u8) = (90, 160, 255);
const THEME_B: (u8, u8, u8) = (0, 220, 200);
const WRAP: (u8, u8, u8) = (255, 120, 60);
const WRAP2: (u8, u8, u8) = (255, 60, 160);
const TEXT: (u8, u8, u8) = (205, 210, 220);
const DIM: (u8, u8, u8) = (120, 126, 140);
const OK: (u8, u8, u8) = (80, 220, 140);
const BAD: (u8, u8, u8) = (240, 90, 110);

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    (
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

/// 把三行字模横向拼起来，逐字符按列位置渐变着色
fn draw_logo(x0: u16, y0: u16, ascii: bool) {
    let (l, b, d) = if ascii {
        (&ASCII_L, &ASCII_B, &ASCII_D)
    } else {
        (&GLYPH_L, &GLYPH_B, &GLYPH_D)
    };
    let total: u16 = 36; // 12 * 3
    for row in 0..6u16 {
        let line = format!("{}{}{}", l[row as usize], b[row as usize], d[row as usize]);
        let mut cx = x0;
        for (i, ch) in line.chars().enumerate() {
            let t = (i as f32) / (total.max(2) as f32);
            let c = mix(THEME_A, THEME_B, t);
            // 阴影字符用暗色，制造立体感
            let c = if ch == '╔' || ch == '╗' || ch == '╚' || ch == '╝' || ch == '═' || ch == '+' {
                mix(c, (30, 40, 55), 0.55)
            } else {
                c
            };
            term::goto(cx, y0 + row);
            if ch == ' ' {
                print!(" ");
            } else {
                term::fg(c.0, c.1, c.2);
                print!("{ch}");
                term::reset();
            }
            cx += 1;
        }
    }
    term::flush();
}

/// 启动自检的一项：真的执行，失败打红叉
fn check_line(y: u16, label: &str, ok: bool, detail: &str) {
    term::spin(3, y, "检查中…", 7, 30);
    if ok {
        term::put(3, y, OK, "✓");
        term::put(5, y, TEXT, label);
        if !detail.is_empty() {
            term::put(6 + term::dwidth(label), y, DIM, detail);
        }
    } else {
        term::put(3, y, BAD, "✗");
        term::put(5, y, TEXT, label);
        term::put(6 + term::dwidth(label), y, BAD, detail);
    }
    term::flush();
}

pub struct BootReport {
    pub checks: Vec<(String, bool, String)>,
    pub ok: bool,
}

pub fn boot_animation() -> BootReport {
    let (cols, _rows) = term::size();
    let width = cols.saturating_sub(6).min(58);
    term::clear();

    // 1) 渐变条从左侧生长
    for w in (1..=width).step_by(3) {
        term::gradient(3, 2, w, THEME_A, THEME_B, '━');
        term::sleep_ms(4);
    }
    term::gradient(3, 2, width, THEME_A, THEME_B, '━');

    // 2) 大字
    draw_logo(3, 4, false);
    term::sleep_ms(60);

    // 3) 标题行
    term::typewrite(3, 11, (235, 240, 255), "六本豆 AGENT", 42);
    term::typewrite(3, 12, DIM, "liubendou · kernel skeleton · zero-dependency", 8);

    // 4) 真自检
    let mut checks: Vec<(String, bool, String)> = Vec::new();

    let kernel_len = crate::apex::KERNEL.len();
    let c1 = ("内核槽位 五段物理排序", kernel_len > 0, format!("{kernel_len} B"));
    let scope_ok = guard::parse_scope("10.0.0.0/24").map(|s| s.allows(&guard::Target::parse("10.0.0.42").unwrap())).unwrap_or(false);
    let c2 = ("授权层 签名 grant + 三道门", scope_ok, String::new());
    let st = store::Store::open(crate::data_path("evidence.jsonl"));
    let (ev_ok, ev_detail) = match &st {
        Ok(s) => (true, format!("{} 条已存", s.len)),
        Err(e) => (false, format!("{e}")),
    };
    let c3 = ("证据库 追加式 JSONL", ev_ok, ev_detail);
    let ttl = store::L3::ttl_for("recon.dns.resolve");
    let c4 = ("缓存层 L1/L2/L3", ttl == 600, format!("dns ttl {ttl}s"));

    for (i, (label, ok, detail)) in [c1, c2, c3, c4].iter().enumerate() {
        check_line(15 + i as u16, label, *ok, detail);
        checks.push((label.to_string(), *ok, detail.clone()));
    }

    // 5) 装配
    term::spin(3, 20, "装配内核…", 16, 38);
    term::put(3, 20, OK, "✓");
    term::put(5, 20, TEXT, &format!("内核就绪 {} B｜{} 个工具｜规则 kernel-1", kernel_len, TOOLS_TOTAL));
    term::flush();

    term::gradient(3, 22, width, WRAP, WRAP2, '─');
    term::sleep_ms(220);

    let ok = checks.iter().all(|(_, o, _)| *o);
    BootReport { checks, ok }
}

// ───────────────────────── 主界面 ─────────────────────────

pub struct App {
    pub transcript: Vec<String>,
    pub grant_line: String,
    pub audit_ok: bool,
    pub last_status: String,
}

impl App {
    pub fn new(report: &BootReport) -> Self {
        let mut t = Vec::new();
        if report.ok {
            t.push("自检全部通过，可以开始。输入 /help 看命令。".to_string());
        } else {
            for (label, ok, detail) in &report.checks {
                if !ok {
                    t.push(format!("自检未通过：{label} — {detail}"));
                }
            }
            t.push("部分自检未通过，功能可能受限。输入 /help 看命令。".to_string());
        }
        App {
            transcript: t,
            grant_line: "未签发授权".to_string(),
            audit_ok: true,
            last_status: String::new(),
        }
    }

    fn push(&mut self, s: impl Into<String>) {
        for line in s.into().lines() {
            self.transcript.push(line.to_string());
        }
    }

    pub fn draw(&self) {
        let (cols, rows) = term::size();
        term::clear();
        let width = cols.saturating_sub(4);

        // 顶部状态栏
        term::gradient(2, 1, width, THEME_A, THEME_B, '━');
        let title = format!("六本豆 AGENT v{}", env!("CARGO_PKG_VERSION"));
        term::put(2, 2, (235, 240, 255), &title);
        let cache = format!("缓存 —｜审计 {}", if self.audit_ok { "OK" } else { "断链" });
        term::put(4 + term::dwidth(&title), 2, DIM, "│");
        term::put(6 + term::dwidth(&title), 2, DIM, &self.grant_line);
        let gx = 6 + term::dwidth(&title) + term::dwidth(&self.grant_line) + 2;
        if gx + term::dwidth(&cache) < cols {
            term::put(gx, 2, if self.audit_ok { OK } else { BAD }, &cache);
        }
        term::put(2, 3, (40, 46, 60), &"─".repeat(width as usize));

        // 会话流（保留最近 rows-9 行）
        let body_top = 5u16;
        let body_bottom = rows.saturating_sub(5);
        let cap = (body_bottom.saturating_sub(body_top)) as usize;
        let start = self.transcript.len().saturating_sub(cap);
        for (i, line) in self.transcript[start..].iter().enumerate() {
            let y = body_top + i as u16;
            let color = if line.starts_with('✓') {
                OK
            } else if line.starts_with('✗') || line.starts_with("拒绝") {
                BAD
            } else if line.starts_with('豆') {
                (235, 240, 255)
            } else {
                TEXT
            };
            term::put(3, y, color, line);
        }

        // 底部
        term::put(2, rows - 3, (40, 46, 60), &"─".repeat(width as usize));
        term::put(2, rows - 1, DIM, "输入命令（/help 看全部）· /quit 退出");
        term::flush();
    }

    pub fn handle(&mut self, cmd: &str) -> bool {
        let cmd = cmd.trim();
        match cmd {
            "" => {}
            "/quit" | "/q" | "/exit" => return false,
            "/help" | "/h" => {
                self.push("豆> 可用命令");
                self.push("  /grant <范围> <档位>   签发授权，例如 /grant 10.0.0.0/24 recon,fingerprint");
                self.push("  /tools                工具注册表与风险档");
                self.push("  /evidence             证据库条数与哈希链校验");
                self.push("  /audit                审计链状态");
                self.push("  /demo                 跑一遍全链路自检");
                self.push("  /clear                清屏");
                self.push("  /quit                 退出");
            }
            "/tools" => {
                self.push(format!("豆> 已注册 {} 个工具", TOOLS_TOTAL));
                for t in crate::guard::TOOLS {
                    self.push(format!("  {:<24} {:<7} {}", t.id, t.risk.code(), t.evidence_kind));
                }
            }
            "/evidence" => match store::Store::open(crate::data_path("evidence.jsonl")) {
                Ok(s) => match s.verify() {
                    Ok(n) => {
                        self.push(format!("豆> 证据库 {} 条，哈希链校验通过 {n} 条", s.len));
                        self.audit_ok = true;
                    }
                    Err(e) => {
                        self.push(format!("✗ 哈希链异常：{e}"));
                        self.audit_ok = false;
                    }
                },
                Err(e) => self.push(format!("✗ 打不开证据库：{e}")),
            },
            "/audit" => {
                self.push(format!(
                    "豆> 审计链 {}｜上次操作：{}",
                    if self.audit_ok { "完整" } else { "断链" },
                    if self.last_status.is_empty() { "无" } else { &self.last_status }
                ));
            }
            "/clear" => self.transcript.clear(),
            "/demo" => {
                self.push("豆> 全链路自检（摘要）");
                let mut a = crate::apex::Assembly::new();
                a.push(crate::apex::SLOT_POLICY, "policy", format!("授权范围 10.0.0.0/24｜档位 recon,fingerprint"));
                let r = a.render();
                self.push(format!("  ✓ 五槽装配 prefix_hash {}", &r.prefix_hash[..16]));
                let mut g = guard::Grant {
                    id: "GR-TUI".into(),
                    scope: guard::parse_scope("10.0.0.0/24").unwrap(),
                    ops: vec!["recon".into()],
                    window: None,
                    rps: 20,
                    approver: "owner".into(),
                    sig: String::new(),
                };
                g.sign(&[7u8; 32]);
                let mut gate = guard::Gate::new(&[7u8; 32]);
                let tool = guard::lookup_tool("recon.port.scan").unwrap();
                let ok = gate.check(tool, &guard::Target::parse("10.0.0.42").unwrap(), &g, 1000);
                let bad = gate.check(tool, &guard::Target::parse("192.168.56.10").unwrap(), &g, 1000);
                self.push(format!("  {} 授权目标放行", if ok.is_ok() { "✓" } else { "✗" }));
                self.push(format!("  {} 越界目标拒绝", if bad.is_err() { "✓" } else { "✗" }));
                self.push(format!("  {} 审计链 {} 条", if gate.audit.verify() { "✓" } else { "✗" }, gate.audit.records.len()));
                self.audit_ok = gate.audit.verify();
                self.last_status = "demo 全链路".into();
            }
            _ if cmd.starts_with("/grant") => {
                let rest = cmd.trim_start_matches("/grant").trim();
                let mut it = rest.splitn(2, ' ');
                let scope = it.next().unwrap_or("").trim();
                let ops = it.next().unwrap_or("recon").trim();
                if scope.is_empty() {
                    self.push("用法：/grant <范围> <档位>，例如 /grant 10.0.0.0/24 recon,fingerprint");
                } else {
                    match guard::parse_scope(scope) {
                        Ok(sc) => {
                            let mut g = guard::Grant {
                                id: format!("GR-{}", crate::now()),
                                scope: sc,
                                ops: ops.split(',').map(|s| s.trim().to_string()).collect(),
                                window: None,
                                rps: 20,
                                approver: "owner".into(),
                                sig: String::new(),
                            };
                            g.sign(&[7u8; 32]);
                            self.grant_line = format!("{}｜{}｜{}", g.id, g.scope.summary(), g.ops.join(","));
                            self.push(format!("✓ 已签发 {} → 范围 {}｜档位 {}", g.id, g.scope.summary(), g.ops.join(",")));
                            self.push(format!("  签名 {}", &g.sig[..16]));
                            self.last_status = format!("签发 {}", g.id);
                        }
                        Err(e) => self.push(format!("✗ {e}")),
                    }
                }
            }
            other => self.push(format!("豆> 未识别：{other}（试 /help）")),
        }
        true
    }
}

/// 纯文本预览字模（不带 ANSI），用于在管道或 CI 里核对形状
pub fn logo_preview(ascii: bool) -> String {
    let (l, b, d) = if ascii {
        (&ASCII_L, &ASCII_B, &ASCII_D)
    } else {
        (&GLYPH_L, &GLYPH_B, &GLYPH_D)
    };
    let mut s = String::new();
    for row in 0..6 {
        s.push_str(&format!("{}{}{}\n", l[row], b[row], d[row]));
    }
    s.push_str("\n六本豆 AGENT  ·  LBD\n");
    s
}

pub fn run(ascii: bool) -> anyhow::Result<()> {
    if !term::init() {
        println!("当前终端不支持 ANSI 虚拟终端，改用纯文本模式。");
        return Ok(());
    }
    term::alt_enter();
    let report = boot_animation();
    let _ = ascii;
    let mut app = App::new(&report);

    loop {
        app.draw();
        let (_, rows) = term::size();
        term::goto(3, rows - 2);
        term::fg(THEME_B.0, THEME_B.1, THEME_B.2);
        print!("豆> ");
        term::reset();
        let _ = io::stdout().flush();

        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }
        if !app.handle(&line) {
            break;
        }
    }

    term::alt_leave();
    Ok(())
}
