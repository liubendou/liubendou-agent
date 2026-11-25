//! 证据库 + 三层缓存。
//!
//! 证据：追加式 JSONL + 哈希链（每条含上一条 hash），任何篡改都能被发现。
//! 缓存：L1 上游前缀命中度量 / L2 本地上下文折叠 / L3 工具结果内容寻址缓存。

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ───────────────────────── 证据库 ─────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub id: u64,
    pub kind: String,
    pub tool: String,
    pub target: String,
    pub payload_hash: String,
    pub payload: String,
    pub prev_hash: String,
    pub hash: String,
    pub ts: u64,
}

pub struct Store {
    path: PathBuf,
    next_id: u64,
    last_hash: String,
    index: BTreeMap<String, Vec<u64>>,
    pub len: usize,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut s = Store {
            path,
            next_id: 1,
            last_hash: "genesis".into(),
            index: BTreeMap::new(),
            len: 0,
        };
        if s.path.exists() {
            let text = fs::read_to_string(&s.path)?;
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let e: Evidence = serde_json::from_str(line)?;
                s.next_id = s.next_id.max(e.id + 1);
                s.last_hash = e.hash.clone();
                s.len += 1;
                s.index_entry(&e);
            }
        }
        Ok(s)
    }

    fn index_entry(&mut self, e: &Evidence) {
        for term in [e.kind.clone(), e.tool.clone(), e.target.clone()] {
            for tok in term.split(['.', '/', ':', '-', ' ']).filter(|t| t.len() > 1) {
                let v = self.index.entry(tok.to_lowercase()).or_default();
                if v.last() != Some(&e.id) {
                    v.push(e.id);
                }
            }
        }
    }

    pub fn append(
        &mut self,
        kind: &str,
        tool: &str,
        target: &str,
        payload: &str,
        ts: u64,
    ) -> anyhow::Result<Evidence> {
        let payload_hash = blake3::hash(payload.as_bytes()).to_hex().to_string();
        let id = self.next_id;
        let mut h = blake3::Hasher::new();
        h.update(self.last_hash.as_bytes());
        h.update(kind.as_bytes());
        h.update(tool.as_bytes());
        h.update(target.as_bytes());
        h.update(payload_hash.as_bytes());
        h.update(&ts.to_le_bytes());
        let hash = h.finalize().to_hex().to_string();

        let e = Evidence {
            id,
            kind: kind.into(),
            tool: tool.into(),
            target: target.into(),
            payload_hash,
            payload: payload.into(),
            prev_hash: self.last_hash.clone(),
            hash: hash.clone(),
            ts,
        };
        let line = serde_json::to_string(&e)?;
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{line}")?;

        self.next_id += 1;
        self.last_hash = hash;
        self.len += 1;
        self.index_entry(&e);
        Ok(e)
    }

    /// 校验整条哈希链。返回校验通过的记录数。
    pub fn verify(&self) -> anyhow::Result<usize> {
        let text = fs::read_to_string(&self.path)?;
        let mut prev = "genesis".to_string();
        let mut n = 0usize;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let e: Evidence = serde_json::from_str(line)?;
            if e.prev_hash != prev {
                anyhow::bail!("链断于 evidence #{}, prev_hash 不符", e.id);
            }
            let mut h = blake3::Hasher::new();
            h.update(prev.as_bytes());
            h.update(e.kind.as_bytes());
            h.update(e.tool.as_bytes());
            h.update(e.target.as_bytes());
            h.update(e.payload_hash.as_bytes());
            h.update(&e.ts.to_le_bytes());
            if blake3::hash(e.payload.as_bytes()).to_hex().to_string() != e.payload_hash {
                anyhow::bail!("evidence #{} payload 与 payload_hash 不符", e.id);
            }
            if h.finalize().to_hex().to_string() != e.hash {
                anyhow::bail!("evidence #{} 内容与哈希不符", e.id);
            }
            prev = e.hash.clone();
            n += 1;
        }
        Ok(n)
    }

    pub fn search(&self, term: &str) -> Vec<u64> {
        self.index.get(&term.to_lowercase()).cloned().unwrap_or_default()
    }
}

// ───────────────────────── L1：上游前缀命中 ─────────────────────────

#[derive(Default, Debug)]
pub struct L1 {
    pub requests: u64,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub prefix_hashes: Vec<String>,
}

impl L1 {
    pub fn record(&mut self, prefix_hash: &str, prompt_tokens: u64, cached_tokens: u64) {
        self.requests += 1;
        self.prompt_tokens += prompt_tokens;
        self.cached_tokens += cached_tokens.min(prompt_tokens);
        if self.prefix_hashes.last().map(|h| h != prefix_hash).unwrap_or(true) {
            self.prefix_hashes.push(prefix_hash.to_string());
        }
    }

    pub fn hit_rate(&self) -> f64 {
        if self.prompt_tokens == 0 {
            0.0
        } else {
            self.cached_tokens as f64 / self.prompt_tokens as f64
        }
    }

    /// 前缀稳定性：不同前缀哈希数越少越好（理想为 1）
    pub fn prefix_distinct(&self) -> usize {
        self.prefix_hashes.len()
    }
}

// ───────────────────────── L2：本地上下文折叠 ─────────────────────────

pub struct Folder {
    /// 折叠后保留的最大字符数
    pub budget: usize,
    /// 头部固定保留的轮数（不动前缀）
    pub keep_head: usize,
    /// 尾部保留的轮数
    pub keep_tail: usize,
}

impl Default for Folder {
    fn default() -> Self {
        Self { budget: 3000, keep_head: 2, keep_tail: 4 }
    }
}

pub struct FoldResult {
    pub kept: Vec<String>,
    pub folded: usize,
    pub summary: Option<String>,
}

impl Folder {
    /// 只折中段，前后两段原样保留 —— 前缀一个字节不碰。
    pub fn fold(&self, turns: &[String]) -> FoldResult {
        let total: usize = turns.iter().map(|t| t.chars().count()).sum();
        if total <= self.budget || turns.len() <= self.keep_head + self.keep_tail {
            return FoldResult { kept: turns.to_vec(), folded: 0, summary: None };
        }
        let head = &turns[..self.keep_head];
        let tail = &turns[turns.len() - self.keep_tail..];
        let mid = &turns[self.keep_head..turns.len() - self.keep_tail];
        let mut summary = format!("[folded {} turns, {} chars]\n", mid.len(), 
            mid.iter().map(|t| t.chars().count()).sum::<usize>());
        for t in mid.iter().take(6) {
            let first = t.lines().next().unwrap_or("").chars().take(80).collect::<String>();
            summary.push_str(&format!("- {}\n", first));
        }
        summary.push_str("[使用 evidence.read 可按 id 还原任意被折叠内容]");

        let mut kept: Vec<String> = head.to_vec();
        kept.push(summary.clone());
        kept.extend(tail.iter().cloned());
        FoldResult { kept, folded: mid.len(), summary: Some(summary) }
    }
}

// ───────────────────────── L3：工具结果缓存 ─────────────────────────

struct Entry {
    value: String,
    expire: u64,
}

#[derive(Default)]
pub struct L3 {
    entries: BTreeMap<String, Entry>,
    pub hits: u64,
    pub misses: u64,
}

impl L3 {
    /// 内容寻址 key：换授权（scope_id）、换规则版本都会自然失效。
    pub fn key(tool: &str, canonical_input: &str, scope_id: &str, ruleset_ver: &str) -> String {
        let mut h = blake3::Hasher::new();
        h.update(tool.as_bytes());
        h.update(b"\x1f");
        h.update(canonical_input.as_bytes());
        h.update(b"\x1f");
        h.update(scope_id.as_bytes());
        h.update(b"\x1f");
        h.update(ruleset_ver.as_bytes());
        h.finalize().to_hex().to_string()
    }

    /// 分级 TTL（秒）
    pub fn ttl_for(tool: &str) -> u64 {
        if tool.contains("dns") {
            600
        } else if tool.contains("port") {
            3600
        } else if tool.contains("service") || tool.contains("http") {
            6 * 3600
        } else if tool.contains("subdomain") {
            24 * 3600
        } else {
            0 // 写类/会话类默认不缓存
        }
    }

    pub fn get(&mut self, key: &str, now: u64) -> Option<String> {
        match self.entries.get(key) {
            Some(e) if e.expire > now => {
                self.hits += 1;
                Some(e.value.clone())
            }
            Some(_) => {
                self.entries.remove(key);
                self.misses += 1;
                None
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    pub fn put(&mut self, key: &str, value: &str, now: u64, ttl: u64) {
        if ttl == 0 {
            return;
        }
        self.entries.insert(key.to_string(), Entry { value: value.to_string(), expire: now + ttl });
    }

    pub fn rate(&self) -> f64 {
        let n = self.hits + self.misses;
        if n == 0 {
            0.0
        } else {
            self.hits as f64 / n as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_chain_detects_tamper() {
        let dir = std::env::temp_dir().join(format!("lbd-test-{}", std::process::id()));
        let p = dir.join("ev.jsonl");
        let _ = fs::remove_file(&p);
        let mut s = Store::open(&p).unwrap();
        s.append("port", "recon.port.scan", "10.0.0.42", "443/tcp open", 1).unwrap();
        s.append("service", "recon.service.identify", "10.0.0.42", "nginx/1.24", 2).unwrap();
        assert_eq!(s.verify().unwrap(), 2);

        let mut text = fs::read_to_string(&p).unwrap();
        text = text.replace("nginx/1.24", "apache/2.4");
        fs::write(&p, text).unwrap();
        assert!(s.verify().is_err(), "篡改后哈希链必须报错");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_keeps_prefix_and_tail() {
        let turns: Vec<String> = (0..20).map(|i| format!("turn-{i} ")).collect();
        let f = Folder { budget: 50, keep_head: 2, keep_tail: 2 };
        let r = f.fold(&turns);
        assert_eq!(r.kept[0], "turn-0 ");
        assert_eq!(r.kept[1], "turn-1 ");
        assert_eq!(r.kept.last().unwrap(), "turn-19 ");
        assert!(r.folded > 0);
    }

    #[test]
    fn l3_key_is_scope_sensitive() {
        let a = L3::key("recon.port.scan", "{\"t\":\"10.0.0.42\"}", "GR-1", "v1");
        let b = L3::key("recon.port.scan", "{\"t\":\"10.0.0.42\"}", "GR-2", "v1");
        assert_ne!(a, b, "换授权必须失效");
    }
}
