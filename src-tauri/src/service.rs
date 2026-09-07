//! service 扩展托管：后端进程启动 / 动态端口 / 探活 / 停止（spec §5）。
//!
//! 一期范围（MVP）：
//! - 懒启动：service 扩展首次打开（read_extension_entry）时启动后端，动态分配 127.0.0.1 空闲端口；
//! - 运行时：检测系统 Node（`node --version`，主版本 ≥ backend.engine.minVersion），复用系统 Node；
//!   按需下载内置运行时依赖扩展市场（§12.7），后续实现；
//! - 探活：TcpStream connect 轮询（未做 HTTP 健康检查路径，后续补）；
//! - 代理转发：非流式走桥 API `service.request`（xhub_api 内 reqwest 转发）；`/svc/<extId>/*`
//!   反向代理与 WebSocket 流式后续实现；
//! - 停止：卸载时调用 `stop_service`（卸载 UI 在 §12.7 接入）。

use crate::extension::{extensions_root, read_manifest, ExtensionManifest};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Manager;

/// 单个 service 扩展的运行时状态
pub struct ServiceRuntime {
    pub port: u16,
    pub ready: bool,
    pub child: Option<std::process::Child>,
}

/// 全局 service 运行时注册表（ext_id → ServiceRuntime）
pub struct ServiceState(pub Mutex<HashMap<String, ServiceRuntime>>);

impl Default for ServiceState {
    fn default() -> Self {
        ServiceState(Mutex::new(HashMap::new()))
    }
}

/// 分配空闲端口。按 host 绑定：
/// - host = 127.0.0.1（默认）：本机回环，安全默认
/// - host = 0.0.0.0 / 局域网地址：对外监听（需 network 权限，由 start_service 校验）
/// bind 0 让 OS 挑端口；固定端口时直接试绑该端口。
fn alloc_port(host: &str, fixed: Option<u16>) -> Result<u16, String> {
    let bind_addr = match fixed {
        Some(p) => format!("{host}:{p}"),
        None => format!("{host}:0"),
    };
    let listener = std::net::TcpListener::bind(&bind_addr).map_err(|e| {
        format!("端口绑定失败 {bind_addr}: {e}")
    })?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    Ok(port)
}

// 运行时解析（系统 Node 优先 + 内置兜底下载）见 runtime.rs

/// 探活：轮询 connect 端口直到成功或超时。
/// host 为通配地址（0.0.0.0/::）时探回环；为具体地址（如局域网 IP）时探该地址——
/// 后端只在该地址监听，盲探 127.0.0.1 会永远失败（探活超时 → serviceReady 恒 false）。
fn probe_ready(port: u16, host: &str, timeout: Duration) -> bool {
    let probe_host = match host {
        "0.0.0.0" | "::" => "127.0.0.1",
        _ => host,
    };
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect((probe_host, port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// 启动 service 扩展后端，返回端口。幂等：已启动则直接复用。
pub fn start_service(
    app: &tauri::AppHandle,
    ext_id: &str,
) -> Result<u16, String> {
    let state = app.state::<ServiceState>();
    {
        let map = state.0.lock().map_err(|e| e.to_string())?;
        if let Some(rt) = map.get(ext_id) {
            return Ok(rt.port);
        }
    }

    let dir = extensions_root(app)?.join(ext_id);
    let manifest: ExtensionManifest = read_manifest(&dir)?;
    let backend = manifest
        .backend
        .as_ref()
        .ok_or_else(|| format!("扩展 {ext_id} 未声明 backend（非 service 扩展）"))?;

    let engine_type = backend
        .engine
        .as_ref()
        .map(|e| e.engine_type.as_str())
        .unwrap_or("node");
    if engine_type != "node" {
        return Err(format!("不支持的运行时引擎类型: {engine_type}"));
    }
    let min_version = backend
        .engine
        .as_ref()
        .and_then(|e| e.min_version.as_deref());
    let strategy = crate::config::load().runtime_strategy;
    let node_exe = crate::runtime::resolve_node(app, min_version, &strategy)?;

    let entry = dir.join(&backend.entry);
    if !entry.is_file() {
        return Err(format!("后端入口不存在: {}", backend.entry));
    }
    let cwd = backend.cwd.as_ref().map(|c| dir.join(c)).unwrap_or_else(|| dir.clone());

    // 监听主机解析 + 对外监听的安全门控：
    // - 默认/127.0.0.1 = 本机回环，任何 service 扩展可用（现状不变）
    // - 0.0.0.0/局域网地址 = 对外开放，必须声明 network 权限且未被用户关闭，否则拒绝启动
    let listen_host = backend.listen_host();
    let external = backend.is_external();
    if external {
        if !manifest.permissions.iter().any(|p| p == "network") {
            return Err(format!(
                "PERMISSION_DENIED: 扩展 {ext_id} 对外监听（host={listen_host}）需要声明 network 权限"
            ));
        }
        if !crate::extension::permission_granted(app, ext_id, "network") {
            return Err(format!(
                "PERMISSION_DENIED: 扩展 {ext_id} 的 network 权限已被用户关闭，无法对外监听"
            ));
        }
    }

    let port = alloc_port(&listen_host, backend.port)?;

    let mut cmd = std::process::Command::new(&node_exe);
    cmd.arg(&entry)
        .current_dir(&cwd)
        .env("PORT", port.to_string())
        .env("XHUB_LISTEN_HOST", &listen_host)
        .env("XHUB_EXT_ID", ext_id)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Err(format!("启动后端失败: {e}"));
        }
    };

    // 先以 ready=false 入库并立即返回端口：netsh 放行（可 1s+）与探活（最长 10s）
    // 都不占用命令线程（read_extension_entry 懒启动路径），后台线程完成后回填 ready
    {
        let mut map = state.0.lock().map_err(|e| e.to_string())?;
        map.insert(
            ext_id.to_string(),
            ServiceRuntime {
                port,
                ready: false,
                child: Some(child),
            },
        );
    }

    let bg_app = app.clone();
    let bg_ext = ext_id.to_string();
    let bg_host = listen_host.clone();
    let bg_program = node_exe.to_string_lossy().to_string();
    std::thread::spawn(move || {
        // 对外监听时放行 Windows 防火墙（非对外不触碰，避免无谓 UAC 提示）
        if external {
            ensure_firewall_rule(&bg_ext, port, &bg_program);
        }
        let ready = probe_ready(port, &bg_host, Duration::from_secs(10));
        if let Ok(mut map) = bg_app.state::<ServiceState>().0.lock() {
            if let Some(rt) = map.get_mut(&bg_ext) {
                rt.ready = ready;
            }
        }
        log::info!("service 探活完成: {bg_ext} port={port} ready={ready}");
    });

    log::info!("service 扩展已启动: {ext_id} port={port}（防火墙/探活后台进行中）");
    Ok(port)
}

/// 停止并清理 service 扩展后端进程（卸载 / 宿主退出时调用）
pub fn stop_service(app: &tauri::AppHandle, ext_id: &str) {
    let mut rt = match app.state::<ServiceState>().0.lock() {
        Ok(mut map) => map.remove(ext_id),
        Err(_) => return,
    };
    if let Some(child) = rt.as_mut().and_then(|r| r.child.take()) {
        kill_and_reap(child);
    }
    // 同步移除对外监听放行的防火墙规则，避免规则永久残留
    remove_firewall_rule(ext_id);
    log::info!("service 扩展已停止: {ext_id}");
}

/// kill + 有界回收：TerminateProcess 后子进程应立即终止，但为防异常进程把
/// 宿主退出流程拖死，轮询 try_wait 至多 1s，超时则放弃（进程句柄由系统回收）
fn kill_and_reap(mut child: std::process::Child) {
    let _ = child.kill();
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => break,
        }
    }
}

/// 宿主退出时停止所有 service 后端进程，避免残留
pub fn stop_all(app: &tauri::AppHandle) {
    let state = app.state::<ServiceState>();
    let mut map = match state.0.lock() {
        Ok(m) => m,
        Err(_) => return,
    };
    let ids: Vec<String> = map.keys().cloned().collect();
    for id in ids {
        if let Some(mut rt) = map.remove(&id) {
            if let Some(child) = rt.child.take() {
                kill_and_reap(child);
            }
        }
        remove_firewall_rule(&id);
    }
    log::info!("宿主退出，已停止所有 service 后端进程");
}

/// 已启动 service 的端口（未启动返回 None）
pub fn service_port(app: &tauri::AppHandle, ext_id: &str) -> Option<u16> {
    let state = app.state::<ServiceState>();
    let map = state.0.lock().ok()?;
    map.get(ext_id).map(|rt| rt.port)
}

/// 防火墙规则名（不含端口：同扩展重启换端口时先删后加幂等覆盖，停止时按名可删）
fn firewall_rule_name(ext_id: &str) -> String {
    format!("x-hub extension {ext_id}")
}

/// 移除扩展的防火墙放行规则（停止/卸载/启动失败时调用），失败仅记录日志
pub(crate) fn remove_firewall_rule(ext_id: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let rule_name = firewall_rule_name(ext_id);
        let out = std::process::Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &format!("name={rule_name}"),
            ])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();
        match out {
            Ok(o) if o.status.success() => {
                log::info!("已移除防火墙规则: {rule_name}");
            }
            Ok(o) => {
                let msg = String::from_utf8_lossy(&o.stderr);
                log::warn!("移除防火墙规则失败（{rule_name}）：{msg}");
            }
            Err(e) => {
                log::warn!("调用 netsh 失败（{rule_name}）：{e}");
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = ext_id;
    }
}

/// 为对外监听的 service 扩展放行 Windows 防火墙（入站 TCP，按端口 + 程序定向）。
/// 失败不阻塞启动：仅记录日志（可能因非管理员权限无法写入，提示用户手动放行）。
fn ensure_firewall_rule(ext_id: &str, port: u16, program: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let rule_name = firewall_rule_name(ext_id);
        // netsh add rule 对同名规则是「叠加」而非覆盖：先删后加保证幂等，
        // 避免动态端口每次启动都新增一条规则无限累积
        let _ = std::process::Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &format!("name={rule_name}"),
            ])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();
        let out = std::process::Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={rule_name}"),
                "dir=in",
                "action=allow",
                "protocol=TCP",
                &format!("localport={port}"),
                &format!("program={program}"),
                "profile=private",
            ])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();
        match out {
            Ok(o) if o.status.success() => {
                log::info!("已放行防火墙: {rule_name} (tcp {port}, program={program})");
            }
            Ok(o) => {
                let msg = String::from_utf8_lossy(&o.stderr);
                log::warn!(
                    "防火墙放行失败（{ext_id} 端口 {port}）：{msg}。如需局域网访问请手动放行该端口。"
                );
            }
            Err(e) => {
                log::warn!("调用 netsh 失败（{ext_id} 端口 {port}）：{e}");
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (ext_id, port, program);
    }
}


/// service 是否就绪
pub fn service_ready(app: &tauri::AppHandle, ext_id: &str) -> bool {
    let state = app.state::<ServiceState>();
    let map = state.0.lock().ok();
    map.and_then(|m| m.get(ext_id).map(|rt| rt.ready))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_port_binds_loopback_and_external() {
        let port = alloc_port("127.0.0.1", None).unwrap();
        assert!(port > 0);
        // 对外监听也能分配
        let ext = alloc_port("0.0.0.0", None).unwrap();
        assert!(ext > 0);
    }

    #[test]
    fn alloc_port_fixed_port_binds_that_port() {
        let p = alloc_port("127.0.0.1", None).unwrap();
        // 占住该端口后，同一端口重复绑定应失败（冲突检测）
        let _guard = std::net::TcpListener::bind(("127.0.0.1", p)).unwrap();
        assert!(alloc_port("127.0.0.1", Some(p)).is_err());
    }

    #[test]
    fn backend_spec_external_detection() {
        let mk = |host: Option<String>| crate::extension::BackendSpec {
            entry: "s.js".into(),
            engine: None,
            cwd: None,
            port: None,
            host,
            health: None,
        };
        assert!(!mk(None).is_external());
        assert!(!mk(Some("127.0.0.1".into())).is_external());
        assert!(!mk(Some("localhost".into())).is_external());
        assert!(mk(Some("0.0.0.0".into())).is_external());
        assert!(mk(Some("192.168.1.5".into())).is_external());
        assert_eq!(mk(None).listen_host(), "127.0.0.1");
        assert_eq!(mk(Some("0.0.0.0".into())).listen_host(), "0.0.0.0");
    }
}
