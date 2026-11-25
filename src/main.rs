//! 六本豆 Agent（liubendou）—— 骨架首版 CLI。
//!
//! 四命令：doctor / grant / run / demo（默认）。
//! 设计原则：授权检查在工具执行**之前**，且由代码读签名 grant；不靠叙事。

mod apex;
mod guard;
mod store;
mod term;
mod tools;
mod tui;
mod install;
mod launcher;
mod proxy;

/// 注册工具总数（UI 显示用）
pub const TOOLS_TOTAL: usize = guard::TOOLS.len();

use std::time::{SystemTime, UNIX_EPOCH};

use guard::{lookup_tool, parse_scope, Gate, Grant, Target, TOOLS};
use store::{Folder, Store, L1, L3};

const SECRET: [u8; 32] = [7u8; 32];
const RULESET: &str = "kernel-1";
const SCOPE_DEMO: &str = "10.0.0.0/24, *.lab.internal, -10.0.0.7";

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 传输层 + 重试层入口
fn proxy_cmd(args: &[String]) -> anyhow::Result<()> {
    let mut port: u16 = 8123;
    let mut upstream = String::from("https://api.deepseek.com/v1");
    let mut is_probe = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if i + 1 < args.len() {
                    port = args[i + 1].parse().unwrap_or(8123);
                    i += 2;
                    continue;
                }
            }
            "--upstream" => {
                if i + 1 < args.len() {
                    upstream = args[i + 1].clone();
                    i += 2;
                    continue;
                }
            }
            "--probe" => {
                is_probe = true;
                if i + 1 < args.len() && args[i + 1].starts_with("http") {
                    upstream = args[i + 1].clone();
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if is_probe {
        return proxy::probe(&upstream);
    }
    proxy::serve(port, upstream)
}

fn sens_str(s: apex::Sensitivity) -> &'static str {
    match s {
        apex::Sensitivity::High => "high",
        apex::Sensitivity::Medium => "medium",
        apex::Sensitivity::Low => "low",
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "" => launcher::run(&args),
        "ui" => tui::run(false),
        "tui" => tui::run(true),
        "cli" => interactive(),
        "install" | "--install" => install::install(),
        "uninstall" | "--uninstall" => install::uninstall(),
        "doctor" => doctor(),
        "grant" => grant_issue(&args[1..]),
        "run" => run(&args[1..]),
        "tools" => tools_list(),
        "demo" => demo(),
        "proxy" => proxy_cmd(&args[1..]),
        "tuitest" => term::selftest(),
        "logopreview" => {
            let ascii = args.get(1).map(|s| s == "ascii").unwrap_or(false);
            print!("{}", tui::logo_preview(ascii));
            Ok(())
        }
        "-h" | "--help" | "help" => {
            help();
            Ok(())
        }
        "-V" | "--version" => {
            println!("六本豆 Agent (liubendou) {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => {
            eprintln!("未知命令：{other}\n");
            help();
            std::process::exit(2);
        }
    }
}

/// 数据目录：exe 同级的 liubendou-data（便携，不污染 cwd）
fn data_dir() -> std::path::PathBuf {
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let d = base.join("liubendou-data");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn data_path(name: &str) -> std::path::PathBuf {
    data_dir().join(name)
}

fn pause() {
    use std::io::Write;
    print!("\n按回车继续…");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

fn help() {
    println!("六本豆 Agent (liubendou) {}", env!("CARGO_PKG_VERSION"));
    println!("用法：");
    println!("  liubendou                      TUI 全屏界面（双击快捷方式即此）");
    println!("  liubendou install              安装到 %LOCALAPPDATA% 并建桌面/开始菜单快捷方式");
    println!("  liubendou uninstall            卸载");
    println!("  liubendou tui                  强制用 ASCII 字模启动 TUI");
    println!("  liubendou cli                  退回旧版纯文本菜单");
    println!("  liubendou logopreview [ascii]  打印启动字模");
    println!("  liubendou --version | --help");
    println!("  liubendou demo                 全链路自检：装配/授权/证据/拒答/缓存/审计");
    println!("  liubendou doctor               环境与内核体检");
    println!("  liubendou tools                工具注册表（含风险档位）");
    println!("  liubendou grant issue [--targets SPEC] [--ops a,b] [--rps N] [--out FILE]");
    println!("  liubendou run --grant FILE --target TARGET [--tool ID] [--task \"...\"]");
    println!("数据目录：{}", data_dir().display());
}

fn tools_list() -> anyhow::Result<()> {
    println!("{:<24} {:<7} 证据类型", "工具 ID", "风险档");
    for t in TOOLS {
        println!("{:<24} {:<7} {}", t.id, t.risk.code(), t.evidence_kind);
    }
    Ok(())
}

fn interactive() -> anyhow::Result<()> {
    use std::io::Write;
    loop {
        println!();
        println!("═══ 六本豆 Agent v{} ═══", env!("CARGO_PKG_VERSION"));
        println!(" 1) 全链路自检 demo");
        println!(" 2) 环境体检 doctor");
        println!(" 3) 工具注册表");
        println!(" 4) 签发授权 grant（默认范围）");
        println!(" 5) 执行一次工具调用（需先用 4 生成 grant 文件）");
        println!(" h) 帮助   q) 退出");
        print!("选择> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line)? == 0 {
            return Ok(());
        }
        match line.trim() {
            "1" => {
                demo()?;
                pause();
            }
            "2" => {
                doctor()?;
                pause();
            }
            "3" => {
                tools_list()?;
                pause();
            }
            "4" => {
                let empty: Vec<String> = vec![];
                grant_issue(&empty)?;
                pause();
            }
            "5" => {
                println!("先用 4 生成 grant 文件，再执行：");
                println!("  liubendou run --grant liubendou-data\\grant.txt --target 10.0.0.42 --tool recon.port.scan");
                pause();
            }
            "h" | "H" | "help" => {
                help();
                pause();
            }
            "q" | "Q" | "quit" | "exit" => return Ok(()),
            "" => return Ok(()),
            other => println!("未知选择：{other}"),
        }
    }
}

// ───────────────────────── doctor ─────────────────────────

fn doctor() -> anyhow::Result<()> {
    println!("六本豆 Agent · doctor");
    println!("  版本        {}", env!("CARGO_PKG_VERSION"));
    println!("  规则版本    {RULESET}");
    println!("  内核槽位    kernel={} policy={} task={} persona={} context={}",
        apex::SLOT_KERNEL, apex::SLOT_POLICY, apex::SLOT_TASK, apex::SLOT_PERSONA, apex::SLOT_CONTEXT);
    println!("  冻结边界    order <= {} 不可被任务层覆盖", apex::FROZEN_MAX);
    println!("  工具注册    {} 个（read/touch/write/exec 四档）", TOOLS.len());
    println!("  内核字节    {} B", apex::KERNEL.len());
    Ok(())
}

// ───────────────────────── grant ─────────────────────────

fn grant_issue(args: &[String]) -> anyhow::Result<()> {
    let mut targets = SCOPE_DEMO.to_string();
    let mut ops = vec!["recon".to_string(), "fingerprint".to_string()];
    let mut rps = 20u32;
    let mut id = format!("GR-{}", now());
    let mut approver = "owner".to_string();
    let mut out: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let k = args[i].as_str();
        if !k.starts_with("--") {
            i += 1;
            continue;
        }
        let v = args.get(i + 1);
        match (k, v) {
            ("--targets", Some(x)) => targets = x.clone(),
            ("--ops", Some(x)) => ops = x.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            ("--rps", Some(x)) => rps = x.parse()?,
            ("--id", Some(x)) => id = x.clone(),
            ("--approver", Some(x)) => approver = x.clone(),
            ("--out", Some(x)) => out = Some(x.clone()),
            ("--help", _) => {
                println!("liubendou grant issue --targets SPEC --ops a,b --rps N --id ID --approver NAME --out FILE");
                return Ok(());
            }
            _ => {}
        }
        i += 2;
    }

    let mut g = Grant {
        id: id.clone(),
        scope: parse_scope(&targets)?,
        ops,
        window: None,
        rps,
        approver,
        sig: String::new(),
    };
    g.sign(&SECRET);

    let line = format!(
        "v1\t{}\t{}\t{}\t{}\t{}\t{}",
        g.id, g.scope.raw, g.ops.join(","), g.rps, g.approver, g.sig
    );
    match out {
        Some(p) => {
            if let Some(dir) = std::path::Path::new(&p).parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&p, &line)?;
            println!("grant {id} 已签发 → {p}");
        }
        None => println!("{line}"),
    }
    println!("  范围   {}", g.scope.summary());
    println!("  操作档 {}", g.ops.join(","));
    println!("  签名   {}", &g.sig[..16.min(g.sig.len())]);
    Ok(())
}

fn load_grant(path: &str) -> anyhow::Result<Grant> {
    let line = std::fs::read_to_string(path)?;
    let line = line.trim();
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 7 || f[0] != "v1" {
        anyhow::bail!("grant 文件格式不符：{path}");
    }
    Ok(Grant {
        id: f[1].to_string(),
        scope: parse_scope(f[2])?,
        ops: f[3].split(',').map(|s| s.to_string()).collect(),
        window: None,
        rps: f[4].parse()?,
        approver: f[5].to_string(),
        sig: f[6].to_string(),
    })
}

// ───────────────────────── run ─────────────────────────

fn run(args: &[String]) -> anyhow::Result<()> {
    let mut grant_path: Option<String> = None;
    let mut target = String::new();
    let mut tool_id = "recon.port.scan".to_string();
    let mut task = "对目标做端口与服务识别".to_string();

    let mut i = 0;
    while i < args.len() {
        let k = args[i].as_str();
        if !k.starts_with("--") {
            i += 1;
            continue;
        }
        let v = args.get(i + 1);
        match (k, v) {
            ("--grant", Some(x)) => grant_path = Some(x.clone()),
            ("--target", Some(x)) => target = x.clone(),
            ("--tool", Some(x)) => tool_id = x.clone(),
            ("--task", Some(x)) => task = x.clone(),
            _ => {}
        }
        i += 2;
    }

    let gp = grant_path.ok_or_else(|| anyhow::anyhow!("缺少 --grant FILE"))?;
    if target.is_empty() {
        anyhow::bail!("缺少 --target TARGET");
    }
    let grant = load_grant(&gp)?;
    let meta = lookup_tool(&tool_id).ok_or_else(|| anyhow::anyhow!("未注册工具：{tool_id}"))?;
    let t = Target::parse(&target)?;

    let mut gate = Gate::new(&SECRET);
    let mut a = apex::Assembly::new();
    a.push(apex::SLOT_POLICY, "policy",
        format!("授权 {}｜范围 {}\n操作档 {}\n速率 {} rps", grant.id, grant.scope.summary(), grant.ops.join(","), grant.rps));
    a.push(apex::SLOT_TASK, "task", apex::synthesize(&task, &grant.ops, &grant.scope.raw, grant.rps).render());
    let r = a.render();
    println!("prefix_hash {}｜约 {} tokens", &r.prefix_hash[..16], a.est_tokens());

    match gate.check(meta, &t, &grant, now()) {
        Ok(risk) => {
            println!("允许 {} → {}（档位 {}）", meta.id, t.text(), risk.code());
            let mut st = Store::open(data_path("evidence.jsonl"))?;
            let e = st.append(meta.evidence_kind, meta.id, &t.text(), "[骨架期占位：真实探测在 S1 接入]", now())?;
            println!("证据 #{id} 落盘，链校验 {} 条", st.verify()?, id = e.id);
        }
        Err(d) => println!("拒绝 {} → {}｜{}：{}", meta.id, t.text(), d.code, d.detail),
    }
    println!("审计链 {} 条，完整={}", gate.audit.records.len(), gate.audit.verify());
    Ok(())
}

// ───────────────────────── demo ─────────────────────────

fn demo() -> anyhow::Result<()> {
    let mut o = String::new();
    o.push_str("═══ 六本豆 Agent · 骨架自检 ═══\n\n");

    let ops = vec!["recon".to_string(), "fingerprint".to_string()];
    let raw_task = "对 lab 网段做存活与端口盘点，产出资产清单和验证记录";

    // 1) 五槽装配
    let mut a = apex::Assembly::new();
    a.push(apex::SLOT_POLICY, "policy",
        format!("授权范围 {SCOPE_DEMO}\n操作档 {}\n速率 20 rps", ops.join(",")));
    a.push(apex::SLOT_TASK, "task", apex::synthesize(raw_task, &ops, SCOPE_DEMO, 20).render());
    a.push(apex::SLOT_PERSONA, "persona", "六本豆 Agent：执行优先，结论必须可验证。".to_string());
    a.push(apex::SLOT_CONTEXT, "context", format!("工具 {} 个｜规则 {RULESET}", TOOLS.len()));
    let r = a.render();
    o.push_str(&format!("[1] 五槽装配\n    冻结前缀 {} B｜变化尾段 {} B｜prefix_hash {}\n    约 {} tokens｜内核 {} B\n\n",
        r.prefix.len(), r.tail.len(), &r.prefix_hash[..16], a.est_tokens(), apex::KERNEL.len()));

    // 2) 未授权目标
    let mut grant = Grant {
        id: "GR-DEMO-1".into(),
        scope: parse_scope(SCOPE_DEMO)?,
        ops: ops.clone(),
        window: None,
        rps: 20,
        approver: "owner".into(),
        sig: String::new(),
    };
    grant.sign(&SECRET);
    let mut gate = Gate::new(&SECRET);
    let scan = lookup_tool("recon.port.scan").unwrap();
    let off = Target::parse("192.168.56.10")?;
    o.push_str("[2] 越界目标（自动化测试用例）\n");
    match gate.check(scan, &off, &grant, now()) {
        Ok(_) => o.push_str("    !! 越界目标被放行，授权层失效\n\n"),
        Err(d) => o.push_str(&format!("    拒绝 {}：{}\n\n", d.code, d.detail)),
    }

    // 3) 授权目标跑通 + 证据链
    let on = Target::parse("10.0.0.42")?;
    o.push_str("[3] 授权目标执行\n");
    match gate.check(scan, &on, &grant, now()) {
        Ok(risk) => {
            o.push_str(&format!("    允许 {} → {}（档位 {}）\n", scan.id, on.text(), risk.code()));
            let mut st = Store::open(data_path("evidence.jsonl"))?;
            let e1 = st.append(scan.evidence_kind, scan.id, &on.text(), "443/tcp open", now())?;
            let e2 = st.append("service", "recon.service.identify", &on.text(), "nginx/1.24.0", now())?;
            o.push_str(&format!("    证据 #{}/#{} 落盘，哈希链校验 {} 条通过｜库内 {} 条\n",
                e1.id, e2.id, st.verify()?, st.len));
            o.push_str(&format!("    按 'port' 检索到 id {:?}\n\n", st.search("port")));
        }
        Err(d) => o.push_str(&format!("    拒绝 {}：{}\n\n", d.code, d.detail)),
    }

    // 4) 提权尝试（工具档位 > 授权档位）
    let ex = lookup_tool("exec.session.open").unwrap();
    o.push_str("[4] 档位提权（exec 工具 vs 仅 recon+fingerprint 授权）\n");
    match gate.check(ex, &on, &grant, now()) {
        Ok(_) => o.push_str("    !! 提权成功，授权层失效\n\n"),
        Err(d) => o.push_str(&format!("    拒绝 {}：{}\n\n", d.code, d.detail)),
    }

    // 5) 拒答 → 状态转移
    let samples = [
        "抱歉，我不能协助进行端口扫描，这违反了我的安全准则。",
        "I'm sorry, but I can't help with that request.",
        "建议您通过合法渠道进行授权测试。",
    ];
    o.push_str("[5] 拒答检测与状态转移\n");
    for s in samples {
        match apex::detect_refusal(s, false) {
            Some(v) => o.push_str(&format!("    [{}] cue=\"{}\" structural={} → {}\n",
                v.kind.code(), v.cue, v.structural, apex::neutralize(&v, 3, &ops).replace('\n', "｜"))),
            None => o.push_str("    未检出拒答\n"),
        }
    }
    o.push_str("\n");

    // 6) 模型路由
    let cfg = apex::RouterCfg::default();
    o.push_str("[6] 模型路由（按敏感度分档）\n");
    for t in ["盘点资产清单", "验证这个漏洞是否可复现", "打点拿站并横向到域控"] {
        let s = apex::classify(t);
        let rt = apex::route(s, &cfg);
        o.push_str(&format!("    {:<20} → {}（{}）\n", t, rt.model, sens_str(s)));
    }
    o.push_str("\n");

    // 7) 三层缓存
    let mut l1 = L1::default();
    l1.record(&r.prefix_hash, 8200, 7400);
    l1.record(&r.prefix_hash, 8400, 7600);
    l1.record("different-prefix", 8000, 0);
    o.push_str("[7] 缓存三层\n");
    o.push_str(&format!("    L1 上游前缀：{}/{} 命中，hit_rate {:.1}%，不同前缀 {} 个\n",
        l1.cached_tokens, l1.prompt_tokens, l1.hit_rate() * 100.0, l1.prefix_distinct()));

    let turns: Vec<String> = (0..40)
        .map(|i| format!("第 {i} 轮工具输出：10.0.0.{i} 开放 443/80/22，服务指纹 nginx/1.24.0、OpenSSH_9.6，响应头含 Server 与 X-Powered-By，已记入证据 #{i}"))
        .collect();
    let fr = Folder::default().fold(&turns);
    o.push_str(&format!("    L2 折叠：{} 轮中折 {} 轮，保留首尾，前缀零改动\n", turns.len(), fr.folded));

    let mut l3 = L3::default();
    let k = L3::key("recon.port.scan", "{\"target\":\"10.0.0.42\"}", &grant.id, RULESET);
    let ttl = L3::ttl_for("recon.port.scan");
    l3.put(&k, "443/tcp open", now(), ttl);
    let _ = l3.get(&k, now());
    let _ = l3.get(&L3::key("recon.port.scan", "{\"target\":\"10.0.0.42\"}", "GR-OTHER", RULESET), now());
    o.push_str(&format!("    L3 内容寻址：ttl {}s，命中率 {:.0}%（换 grant 即失效）\n\n", ttl, l3.rate() * 100.0));

    // 8) 审计链
    let n = gate.audit.records.len();
    let last = gate.audit.records.last().map(|x| x.hash[..16].to_string()).unwrap_or_default();
    o.push_str(&format!("[8] 审计哈希链：{} 条记录，完整={}，末链 {}\n", n, gate.audit.verify(), last));
    for rec in &gate.audit.records {
        o.push_str(&format!("    #{}. {:<24} {:<14} {}\n", rec.seq, rec.tool, rec.target, rec.decision));
    }

    print!("{o}");
    Ok(())
}
