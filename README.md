# 六本豆 Agent

一个 Rust 编写的本地命令行 Agent：负责安装、启动、进程守护、代理与终端交互。

## 模块

- `main.rs`：命令入口与调度
- `launcher.rs` / `install.rs`：启动与安装
- `guard.rs`：进程守护
- `proxy.rs` / `term.rs` / `tui.rs`：代理、终端与 TUI
- `store.rs` / `apex.rs`：本地存储与扩展

## 构建

```bash
cargo build --release
```

## 许可证

MIT
