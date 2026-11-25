//! 授权层：Scope DSL · 签名 grant · 三道门 · 审计哈希链
//!
//! 与 dsh-purge 的根本差别：它是"叙事层说已授权，执行层不读 grant"（drill-grant 只写
//! `{granted:true}`，lib/redteam/* 对 drill|grant 匹配数为 0）；这里 grant 必须被代码读，
//! 校验点在**每一次**工具调用之前，且范围表达式由签发方签名，运行期不可改。

use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Ip(Ipv4Addr),
    Domain(String),
}

impl Target {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let t = s.trim();
        if let Ok(ip) = t.parse::<Ipv4Addr>() {
            return Ok(Target::Ip(ip));
        }
        if t.is_empty() || t.contains(' ') {
            anyhow::bail!("非法目标：{t}");
        }
        Ok(Target::Domain(t.to_lowercase()))
    }

    pub fn text(&self) -> String {
        match self {
            Target::Ip(ip) => ip.to_string(),
            Target::Domain(d) => d.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub raw: String,
    /// (network 前缀, mask 位数)
    pub cidrs: Vec<(u32, u8)>,
    /// 通配域名（存 *.x.com 中的 x.com）
    pub domains: Vec<String>,
    /// 精确域名
    pub exact_domains: Vec<String>,
    pub exclude_cidrs: Vec<(u32, u8)>,
    pub exclude_domains: Vec<String>,
}

pub fn parse_scope(spec: &str) -> anyhow::Result<Scope> {
    let mut sc = Scope { raw: spec.trim().to_string(), ..Default::default() };
    for part in spec.split(&[',', ';', '\n'][..]) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let (neg, body) = match p.strip_prefix('-') {
            Some(rest) => (true, rest.trim()),
            None => (false, p),
        };
        if body.contains('/') {
            let (net, mask) = parse_cidr(body)?;
            if neg {
                sc.exclude_cidrs.push((net, mask));
            } else {
                sc.cidrs.push((net, mask));
            }
        } else if let Ok(ip) = body.parse::<Ipv4Addr>() {
            let net = u32::from(ip);
            if neg {
                sc.exclude_cidrs.push((net, 32));
            } else {
                sc.cidrs.push((net, 32));
            }
        } else if let Some(rest) = body.strip_prefix("*.") {
            let d = rest.to_lowercase();
            if neg {
                sc.exclude_domains.push(d);
            } else {
                sc.domains.push(d);
            }
        } else {
            let d = body.to_lowercase();
            if neg {
                sc.exclude_domains.push(d);
            } else {
                sc.exact_domains.push(d);
            }
        }
    }
    if sc.cidrs.is_empty() && sc.domains.is_empty() && sc.exact_domains.is_empty() {
        anyhow::bail!("范围为空，拒绝签发空授权");
    }
    Ok(sc)
}

fn parse_cidr(s: &str) -> anyhow::Result<(u32, u8)> {
    let (ip_s, mask_s) = s.split_once('/').ok_or_else(|| anyhow::anyhow!("非法 CIDR: {s}"))?;
    let ip: Ipv4Addr = ip_s.trim().parse()?;
    let mask: u8 = mask_s.trim().parse()?;
    if mask > 32 {
        anyhow::bail!("掩码越界: {mask}");
    }
    Ok((u32::from(ip), mask))
}

fn mask_of(bits: u8) -> u32 {
    if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    }
}

impl Scope {
    pub fn allows(&self, t: &Target) -> bool {
        match t {
            Target::Ip(ip) => {
                let v = u32::from(*ip);
                if self.exclude_cidrs.iter().any(|(n, m)| v & mask_of(*m) == *n & mask_of(*m)) {
                    return false;
                }
                self.cidrs.iter().any(|(n, m)| v & mask_of(*m) == *n & mask_of(*m))
            }
            Target::Domain(d) => {
                if self.exclude_domains.iter().any(|x| domain_match(d, x)) {
                    return false;
                }
                self.exact_domains.iter().any(|x| x == d)
                    || self.domains.iter().any(|x| domain_match(d, x))
            }
        }
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (n, m) in &self.cidrs {
            parts.push(format!("{}/{}", Ipv4Addr::from(*n), m));
        }
        for d in &self.domains {
            parts.push(format!("*.{d}"));
        }
        for d in &self.exact_domains {
            parts.push(d.clone());
        }
        let mut s = parts.join(", ");
        let ex: Vec<String> = self
            .exclude_cidrs
            .iter()
            .map(|(n, m)| format!("{}/{}", Ipv4Addr::from(*n), m))
            .chain(self.exclude_domains.iter().cloned())
            .collect();
        if !ex.is_empty() {
            s.push_str(&format!(" | 排除: {}", ex.join(", ")));
        }
        s
    }
}

fn domain_match(d: &str, pattern: &str) -> bool {
    d == pattern || d.ends_with(&format!(".{pattern}"))
}

// ───────────────────────── 签名 grant ─────────────────────────

pub const ALL_OPS: &[&str] = &["recon", "fingerprint", "verify", "exploit", "exec"];

#[derive(Clone, Debug)]
pub struct Grant {
    pub id: String,
    pub scope: Scope,
    pub ops: Vec<String>,
    /// (not_before, not_after)，UNIX 秒；None 表示不设窗口
    pub window: Option<(u64, u64)>,
    pub rps: u32,
    pub approver: String,
    pub sig: String,
}

impl Grant {
    pub fn canonical(&self) -> String {
        format!(
            "v1|id={}|scope={}|ops={}|window={}|rps={}|approver={}",
            self.id,
            self.scope.raw,
            self.ops.join(","),
            match self.window {
                Some((a, b)) => format!("{a}-{b}"),
                None => "none".into(),
            },
            self.rps,
            self.approver
        )
    }

    pub fn sign(&mut self, secret: &[u8; 32]) {
        self.sig = blake3::keyed_hash(secret, self.canonical().as_bytes()).to_hex().to_string();
    }

    pub fn verify(&self, secret: &[u8; 32]) -> bool {
        let expect = blake3::keyed_hash(secret, self.canonical().as_bytes()).to_hex().to_string();
        self.sig == expect
    }

    pub fn accepts_op(&self, op: &str) -> bool {
        self.ops.iter().any(|o| o == op) || self.ops.iter().any(|o| o == "*")
    }
}

// ───────────────────────── 风险分级 → 所需操作 ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Read,
    Touch,
    Write,
    Exec,
}

impl Risk {
    pub fn required_op(&self) -> &'static str {
        match self {
            Risk::Read => "recon",
            Risk::Touch => "fingerprint",
            Risk::Write => "verify",
            Risk::Exec => "exploit",
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Touch => "touch",
            Risk::Write => "write",
            Risk::Exec => "exec",
        }
    }
}

pub struct ToolMeta {
    pub id: &'static str,
    pub risk: Risk,
    pub evidence_kind: &'static str,
}

/// 工具注册表（骨架期先落 read/touch 两档，其余档位在 S2 扩）
pub const TOOLS: &[ToolMeta] = &[
    ToolMeta { id: "recon.subdomain.enum", risk: Risk::Read, evidence_kind: "subdomain" },
    ToolMeta { id: "recon.dns.resolve", risk: Risk::Read, evidence_kind: "dns" },
    ToolMeta { id: "recon.port.scan", risk: Risk::Touch, evidence_kind: "port" },
    ToolMeta { id: "recon.service.identify", risk: Risk::Touch, evidence_kind: "service" },
    ToolMeta { id: "recon.http.probe", risk: Risk::Touch, evidence_kind: "http_evidence" },
    ToolMeta { id: "verify.vuln.check", risk: Risk::Write, evidence_kind: "vuln" },
    ToolMeta { id: "exec.session.open", risk: Risk::Exec, evidence_kind: "access_session" },
];

pub fn lookup_tool(id: &str) -> Option<&'static ToolMeta> {
    TOOLS.iter().find(|t| t.id == id)
}

// ───────────────────────── 三道门 ─────────────────────────

pub struct Denial {
    pub code: &'static str,
    pub detail: String,
}

impl Denial {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }
}

#[derive(Clone, Debug)]
pub struct AuditRec {
    pub seq: u64,
    pub tool: String,
    pub target: String,
    pub decision: String,
    pub prev: String,
    pub hash: String,
    pub ts: u64,
}

#[derive(Default)]
pub struct Audit {
    pub records: Vec<AuditRec>,
}

impl Audit {
    pub fn append(&mut self, tool: &str, target: &str, decision: &str, ts: u64) -> &AuditRec {
        let seq = self.records.len() as u64 + 1;
        let prev = self.records.last().map(|r| r.hash.clone()).unwrap_or_else(|| "genesis".into());
        let mut h = blake3::Hasher::new();
        h.update(prev.as_bytes());
        h.update(tool.as_bytes());
        h.update(target.as_bytes());
        h.update(decision.as_bytes());
        h.update(&ts.to_le_bytes());
        let hash = h.finalize().to_hex().to_string();
        self.records.push(AuditRec {
            seq,
            tool: tool.into(),
            target: target.into(),
            decision: decision.into(),
            prev,
            hash,
            ts,
        });
        self.records.last().unwrap()
    }

    pub fn verify(&self) -> bool {
        let mut prev = "genesis".to_string();
        for r in &self.records {
            if r.prev != prev {
                return false;
            }
            let mut h = blake3::Hasher::new();
            h.update(prev.as_bytes());
            h.update(r.tool.as_bytes());
            h.update(r.target.as_bytes());
            h.update(r.decision.as_bytes());
            h.update(&r.ts.to_le_bytes());
            if h.finalize().to_hex().to_string() != r.hash {
                return false;
            }
            prev = r.hash.clone();
        }
        true
    }
}

pub struct Gate<'a> {
    secret: &'a [u8; 32],
    pub audit: Audit,
}

impl<'a> Gate<'a> {
    pub fn new(secret: &'a [u8; 32]) -> Self {
        Self { secret, audit: Audit::default() }
    }

    /// 三道门：签名 → 时间窗 → 范围 → 操作档位 → 审计落链。
    /// 顺序被写死，调用方无法跳过任何一道。
    pub fn check(
        &mut self,
        tool: &ToolMeta,
        target: &Target,
        grant: &Grant,
        now: u64,
    ) -> Result<Risk, Denial> {
        let decision = (|| -> Result<Risk, Denial> {
            if !grant.verify(self.secret) {
                return Err(Denial::new("grant_signature_invalid", "grant 签名不符，可能被篡改"));
            }
            if let Some((a, b)) = grant.window {
                if now < a || now > b {
                    return Err(Denial::new(
                        "grant_window_expired",
                        format!("当前 {now} 不在授权窗口 {a}-{b} 内"),
                    ));
                }
            }
            if !grant.scope.allows(target) {
                return Err(Denial::new(
                    "scope_out_of_grant",
                    format!("目标 {} 不在 {} 内", target.text(), grant.scope.summary()),
                ));
            }
            let need = tool.risk.required_op();
            if !grant.accepts_op(need) {
                return Err(Denial::new(
                    "op_not_granted",
                    format!("工具 {} 需要操作档 {need}，本 grant 只有 [{}]", tool.id, grant.ops.join(",")),
                ));
            }
            Ok(tool.risk)
        })();

        let d = match &decision {
            Ok(r) => format!("allow:{}", r.code()),
            Err(e) => format!("deny:{}", e.code),
        };
        self.audit.append(tool.id, &target.text(), &d, now);
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> Grant {
        let mut g = Grant {
            id: "GR-DEMO".into(),
            scope: parse_scope("10.0.0.0/24, *.lab.internal, -10.0.0.7").unwrap(),
            ops: vec!["recon".into(), "fingerprint".into()],
            window: None,
            rps: 20,
            approver: "owner".into(),
            sig: String::new(),
        };
        g.sign(&[7u8; 32]);
        g
    }

    #[test]
    fn scope_matching() {
        let g = g();
        assert!(g.scope.allows(&Target::parse("10.0.0.42").unwrap()));
        assert!(!g.scope.allows(&Target::parse("10.0.0.7").unwrap()), "排除项必须生效");
        assert!(!g.scope.allows(&Target::parse("10.0.1.1").unwrap()));
        assert!(g.scope.allows(&Target::parse("a.lab.internal").unwrap()));
        assert!(!g.scope.allows(&Target::parse("evil.example.com").unwrap()));
    }

    #[test]
    fn gates_deny_unauthorized_target() {
        let g = g();
        let mut gate = Gate::new(&[7u8; 32]);
        let t = lookup_tool("recon.port.scan").unwrap();
        let r = gate.check(t, &Target::parse("192.168.1.1").unwrap(), &g, 1000);
        assert!(r.is_err());
        assert_eq!(r.err().unwrap().code, "scope_out_of_grant");
        assert!(gate.audit.verify());
    }

    #[test]
    fn gates_deny_op_escalation() {
        let g = g();
        let mut gate = Gate::new(&[7u8; 32]);
        let t = lookup_tool("exec.session.open").unwrap();
        let r = gate.check(t, &Target::parse("10.0.0.42").unwrap(), &g, 1000);
        assert_eq!(r.err().unwrap().code, "op_not_granted");
    }

    #[test]
    fn tampered_grant_rejected() {
        let mut g = g();
        g.ops.push("exec".into());
        let mut gate = Gate::new(&[7u8; 32]);
        let t = lookup_tool("recon.dns.resolve").unwrap();
        let r = gate.check(t, &Target::parse("10.0.0.42").unwrap(), &g, 1000);
        assert_eq!(r.err().unwrap().code, "grant_signature_invalid");
    }
}
