//! 桌面悬浮球（ADR 0004）：常驻透明置顶小球，仅主窗口隐藏时显示。
//! 单击展开环形快捷菜单、双击切换主窗口（开着则收起）、右键托盘同款菜单；
//! 拖拽记忆位置，开启「贴边自动隐藏」时靠近屏幕边缘松手 → 球心落在屏边，
//! 只露出半个球体（悬停/拖拽/展开时完整滑出，前端按 dock 状态 CSS 平移实现）。
//! 曾用「贴边吸附」（完整贴边停靠），用户反馈从未生效且不需要，已替换。
//! Windows-only：独立透明无边框窗口，与 countdown_window 同模式复用。
//!
//! 几何模型：球态窗口 = BALL_SIZE，菜单态 = MENU_SIZE，均以「球心」（窗口中心）为锚
//! 原子切换（单次 SetWindowPos）。窗口 resize 时 WebView2 内容重排滞后一帧，旧帧按
//! 旧视口渲染会让球先「跳」向窗口移动方向再弹回——前端在开合前后把窗口内容整体
//! 淡出/淡入，把跳动帧掩盖在「球化开成菜单」的过渡里（见 FloatingBallWindow 开合时序）。
//!
//! DPI 自愈（非整数缩放裁切修复）：系统缩放为非标准档位（如 110% → 106 DPI，
//! scale = 1.104166…）时，窗口物理尺寸与 WebView2 CSS 视口的换算存在取整，且悬浮球
//! 窗口大部分时间隐藏——隐藏窗口可能错过 WM_DPICHANGED（或 Win10/远程会话关闭
//! 「拖动时显示窗口内容」时 tao 显式跳过尺寸缩放），导致 tao 缓存的 scale_factor 与
//! 窗口物理尺寸都停留在旧值，而 WebView2 光栅化用窗口实时 DPI → 视口从 100 缩到
//! ~90.6 CSS px，球体最外圈陀螺环（视觉 ~94.8px）左右两侧被窗口边缘裁掉一截。
//! 对策：① 一切几何计算用 GetDpiForWindow 实时取 DPI（与 WebView2 同源，缓存过期
//! 也能算对）；② apply_geometry 用窗口实际 outer_size 推球心并按实时 scale 重设目标
//! 尺寸——任何开合/显示/动作都会顺带把失配修正回来；③ ScaleFactorChanged 事件即时
//! 重算；④ 前端 resize 失配自检兜底（floating_ball_reapply）。

use serde::Serialize;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

use crate::config;

/// 悬浮球窗口 label（App.vue 按此路由到 FloatingBallWindow）
pub const LABEL: &str = "floating-ball";

/// 球态窗口尺寸（逻辑 px：48 中心球体 + 光晕/粒子/陀螺环余量，避免视觉被窗口裁切；
/// 100 = 容纳陀螺环最外圈视觉 94px + 3px 余量）
pub const BALL_SIZE: f64 = 100.0;
/// 环形菜单展开态窗口尺寸（逻辑 px：按钮轨道半径 92 + 按钮 26 → 外沿 118，中心 130 留 12px 余量；
/// 用户反馈 312 太空旷——按钮内沿距球缘 45px，收紧到 260 后空隙约 19px，8 键 hover 仍不重叠）
pub const MENU_SIZE: f64 = 260.0;
/// 球体半径（逻辑 px）：前端 .fb-ball 视觉 48px 直径的半径；
/// 用于默认初始位置留白与贴边自动隐藏的触发判定
const BALL_R: f64 = 24.0;
/// 贴边自动隐藏触发距离（逻辑 px，球心距工作区边缘）：拖拽松手时球缘距屏边
/// 在 DOCK_TRIGGER - BALL_R（24px，约一个球半径）内 → 球心落到屏边，半隐。
/// 再远说明用户是「放」不是「贴」，保持原位
const DOCK_TRIGGER: f64 = 48.0;
/// 停靠判定容差（物理 px）：已存储球心距屏边在该值内即视为该侧停靠态
/// （半隐位置是精确落在屏边的，容差只吸收 DPI 取整误差）
const DOCK_TOL: f64 = 6.0;
/// 默认初始位置留白：球缘距工作区边缘的视觉间距（逻辑 px）
const SNAP_GAP: f64 = 7.0;
/// 环形按钮上限（超过会互相重叠；保存命令与设置页双重钳制）
pub const MAX_BUTTONS: usize = 8;

/// 展开前的窗口位置（物理 px）：收起时恢复，靠边挪位后球能回到原吸附点。
/// 用全局 Mutex 而非 thread_local：拖拽/展开命令是 async（跑在tokio线程池），
/// 主线程的 sync_with_main 也会收拢几何，跨线程共享必须用带锁的静态。
#[cfg(target_os = "windows")]
static PRE_EXPAND_POS: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// 主窗是否处于最小化（与「隐藏到托盘」一样属于视觉不可见 → 球显示）。
/// MAIN_WINDOW_VISIBLE 状态位只覆盖托盘/快捷键的显式 show/hide，点标题栏最小化
/// 不经过那条链——由主窗 Resized 事件检测最小化变化后经 set_main_minimized 更新。
#[cfg(target_os = "windows")]
static MAIN_MINIMIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 主窗最小化状态变化入口（lib.rs 主窗事件钩子调用）：更新状态并联动球显隐
pub fn set_main_minimized(app: &AppHandle, minimized: bool) {
    #[cfg(target_os = "windows")]
    {
        use std::sync::atomic::Ordering;
        if MAIN_MINIMIZED.swap(minimized, Ordering::SeqCst) == minimized {
            return;
        }
        sync_with_main(app);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, minimized);
    }
}

/// 贴边停靠状态：球心落在某侧工作区边缘（该侧球体藏屏外一半）。
/// 前端据此做 CSS 平移：悬停/拖拽/菜单展开时向屏内滑出 BALL_R 完整露出。
#[derive(Debug, Default, Serialize, Clone, Copy)]
pub struct DockState {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

#[derive(Debug, Serialize)]
pub struct FloatingBallState {
    pub enabled: bool,
    /// 贴边自动隐藏（取代旧「贴边吸附」）
    pub auto_hide: bool,
    /// 与主窗口同时显示（主窗可见时球保持常驻）
    pub with_main: bool,
    pub buttons: Vec<String>,
    /// 记忆的球心位置（物理 px，拖拽松手后由后端记忆；None = 从未拖拽过）
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// 当前停靠边（半隐态）：前端悬停露出的平移方向依据
    pub dock: DockState,
    /// 球态窗口逻辑边长（前端 resize 失配自检的期望值之一）
    pub ball_size: f64,
    pub menu_size: f64,
}

/// 停靠判定：存储球心（物理 px）距所在显示器工作区边缘在 DOCK_TOL 内 → 该侧停靠
#[cfg(target_os = "windows")]
fn dock_state(win: &tauri::WebviewWindow, cx: Option<f64>, cy: Option<f64>) -> DockState {
    let (Some(cx), Some(cy)) = (cx, cy) else {
        return DockState::default();
    };
    let Ok(Some(mon)) = win.current_monitor() else {
        return DockState::default();
    };
    let wa = mon.work_area();
    let left = wa.position.x as f64;
    let top = wa.position.y as f64;
    let right = left + wa.size.width as f64;
    let bottom = top + wa.size.height as f64;
    DockState {
        left: (cx - left).abs() <= DOCK_TOL,
        right: (right - cx).abs() <= DOCK_TOL,
        top: (cy - top).abs() <= DOCK_TOL,
        bottom: (bottom - cy).abs() <= DOCK_TOL,
    }
}

/// 窗口实时 DPI 缩放系数：直接查 GetDpiForWindow，不用 tao 缓存的 scale_factor。
/// 缓存过期场景：悬浮球隐藏期间系统缩放变化、窗口错过 WM_DPICHANGED（见模块注释
/// 「DPI 自愈」）——此时缓存 scale 停在旧值，而 WebView2 光栅化用窗口实时 DPI，
/// 两侧换算必须同源才不会裁切。取不到 HWND/失败时回退 tao 缓存（非 Windows 编译
/// 走不到此分支，窗口 API 的调用点都在 #[cfg(target_os = "windows")] 内）。
#[cfg(target_os = "windows")]
fn window_scale(win: &tauri::WebviewWindow) -> f64 {
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    if let Ok(hwnd) = win.hwnd() {
        let dpi = unsafe { GetDpiForWindow(hwnd.0) };
        if dpi > 0 {
            return dpi as f64 / 96.0;
        }
    }
    win.scale_factor().unwrap_or(1.0)
}

// ---------- 生命周期 ----------

/// 启动时预创建悬浮球窗口并隐藏常驻（与 clipboard 浮层同模式，见 ADR 0004「非惰性创建」）。
/// 停用/启用只切窗口显隐，绝不运行时销毁重建——运行时现场创建/销毁 WebView2 窗口
/// 是主线程长任务，曾与悬浮球窗口操作交错导致整窗未响应（WebView2 controller 创建挂起，
/// 见 clipboard.rs::init_overlay_window 与 lib.rs 启动注释的同款坑）。
/// 必须在 autostart-hidden 的 tray::hide_window 之后调用，显隐联动才正确。
pub fn init(app: &AppHandle) {
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = ensure_window(app) {
            log::warn!("悬浮球窗口创建失败: {}", e);
            return;
        }
        if !config::load().floating_ball_enabled {
            // 停用状态：窗口隐藏常驻，设置启用时直接 show 即可
            if let Some(win) = app.get_webview_window(LABEL) {
                let _ = win.hide();
            }
            return;
        }
        sync_with_main(app);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
    }
}

#[cfg(target_os = "windows")]
fn ensure_window(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    let mut builder =
        tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App("index.html".into()))
            .title("悬浮球")
            .inner_size(BALL_SIZE, BALL_SIZE)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(true)
            .additional_browser_args(crate::ADDITIONAL_BROWSER_ARGS);
    // 透明窗口在 Windows 上不能同时启用系统阴影（黑边），与便签/倒计时浮窗一致
    builder = builder.shadow(false);
    let win = builder.build()?;

    place_initial(app, &win);

    // 悬浮球不响应关闭请求（Alt+F4 等）：隐藏即可，窗口常驻复用
    // （非惰性创建，避免每次唤出的窗口创建延迟，见 ADR 0004）
    let handle = app.clone();
    win.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                if let Some(w) = handle.get_webview_window(LABEL) {
                    let _ = w.hide();
                }
            }
            // DPI 变化：tao 会按建议矩形缩放窗口，但悬浮球常驻隐藏，隐藏期间可能错过
            // WM_DPICHANGED（或 Win10/远程会话关闭「拖动时显示窗口内容」时 tao 跳过
            // 尺寸缩放）——收到本事件立即按当前态重算几何，把物理尺寸拉回实时 DPI
            // 对应值（失配会让球体陀螺环被窗口边缘裁掉一截，见 apply_geometry 注释）
            tauri::WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = handle.get_webview_window(LABEL) {
                    apply_geometry(&w, is_expanded(&w), None);
                }
            }
            _ => {}
        }
    });

    log::info!("悬浮球窗口已创建");
    Ok(())
}

/// 初始位置：优先用记忆的球心（物理 px），否则主显示器右下角（球缘距屏边 SNAP_GAP，
/// 与吸附同语义）。球态窗口位置 = 球心 - 半边长。
/// 注意：floating_ball_x/y 存球心坐标（旧版本存的是窗口左上角，升级后首次
/// 位置会偏移一次，拖动一下即按新语义记忆）。
#[cfg(target_os = "windows")]
fn place_initial(app: &AppHandle, win: &tauri::WebviewWindow) {
    let scale = window_scale(win);
    let half = (BALL_SIZE * scale / 2.0).round() as i32;
    let cfg = config::load();
    if let (Some(x), Some(y)) = (cfg.floating_ball_x, cfg.floating_ball_y) {
        if crate::is_position_on_screen(x, y) {
            let _ = win.set_position(PhysicalPosition::new(
                (x - half as f64).round() as i32,
                (y - half as f64).round() as i32,
            ));
            return;
        }
    }
    if let Ok(Some(mon)) = app.primary_monitor() {
        let gap = (SNAP_GAP * scale).round() as i32;
        let ball_r = (BALL_R * scale).round() as i32;
        // 默认右下角：按工作区（扣除任务栏）定位，球缘距工作区右/下各 SNAP_GAP，
        // 否则任务栏在底部时默认位会被任务栏盖住
        let wa = mon.work_area();
        let cx = wa.position.x + wa.size.width as i32 - ball_r - gap;
        let cy = wa.position.y + wa.size.height as i32 - ball_r - gap;
        let _ = win.set_position(PhysicalPosition::new(cx - half, cy - half));
    }
}

/// 与主窗口显隐联动：主窗显示且未开「同显」→ 球隐藏；其余情况 → 球显示。
/// 主窗的所有 show/hide 都走 tray::show_window / hide_window，统一钩到这里。
pub fn sync_with_main(app: &AppHandle) {
    #[cfg(target_os = "windows")]
    {
        let cfg = config::load();
        if !cfg.floating_ball_enabled {
            return;
        }
        let Some(win) = app.get_webview_window(LABEL) else {
            return;
        };
        // 主窗视觉不可见 = 隐藏到托盘 ∨ 最小化
        let main_visible = crate::tray::is_main_window_visible()
            && !MAIN_MINIMIZED.load(std::sync::atomic::Ordering::SeqCst);
        // 球保持显示：主窗不可见 ∨ 设置开启「与主窗口同时显示」
        let keep_ball = !main_visible || cfg.floating_ball_with_main;
        if keep_ball {
            // 曾以菜单态被隐藏时先回到球态几何再显示（隐藏期间收拢，跳动不可见）
            apply_geometry(&win, false, None);
            let _ = win.show();
            // 通知页面复位菜单态（几何已在上面收拢）
            use tauri::Emitter;
            let _ = app.emit_to(LABEL, "floating-ball-shown", ());
        } else {
            let _ = win.hide();
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
    }
}

// ---------- 几何管理 ----------

/// 当前是否处于菜单展开态（按窗口实际尺寸判断，免维护额外状态）
#[cfg(target_os = "windows")]
fn is_expanded(win: &tauri::WebviewWindow) -> bool {
    let scale = window_scale(win);
    win.outer_size()
        .map(|sz| sz.width as f64 > BALL_SIZE * scale * 1.5)
        .unwrap_or(false)
}

/// 以「球心」（窗口中心）为锚调整窗口几何：球态 = BALL_SIZE，菜单态 = MENU_SIZE。
/// 展开时钳制到所在显示器内（空间自适应挪位，收起后自然回到吸附位置）。
///
/// DPI 失配自愈（见模块注释）：球心一律按窗口「实际」outer_size 的一半推算（视觉
/// 球心 = 窗口中心，与换算方式无关），目标尺寸按实时 DPI 重算——窗口物理尺寸偏离
/// 期望（隐藏期间错过 WM_DPICHANGED 等）时，任何一次开合/显示都会被本函数拉回：
/// SetWindowPos 同时修正尺寸 → WM_SIZE → wry 同步 WebView2 bounds → 视口恢复。
/// 换用 tao 缓存 scale 的话，缓存过期时目标尺寸永远算成旧值，失配无法自愈。
/// `scale_override`：前端视口实测 DPI（窗口物理宽 / CSS 视口宽）。窗口 DPI 上下文
/// 过期（隐藏期间错过 WM_DPICHANGED 且 GetDpiForWindow 仍返回旧值）时，
/// window_scale 算出的目标物理尺寸依然偏小 → WebView2 按真实光栅 DPI 渲染，
/// 视口 < 逻辑尺寸，菜单按钮外圈被窗口边缘裁掉。此时以「视口实测」为唯一真相，
/// 前端 checkViewportSync 失配时携带 clientWidth 调 floating_ball_reapply 自愈。
#[cfg(target_os = "windows")]
fn apply_geometry(win: &tauri::WebviewWindow, expanded: bool, scale_override: Option<f64>) {
    let Ok(pos) = win.outer_position() else { return };
    let scale = scale_override.unwrap_or_else(|| window_scale(win));
    let was_expanded = is_expanded(win);

    // 当前球心 = 窗口实际中心；取不到实际尺寸（极端）才退回逻辑换算
    let cur_half = match win.outer_size() {
        Ok(sz) if sz.width > 0 && sz.height > 0 => {
            (sz.width as f64 + sz.height as f64) / 4.0
        }
        _ => (if was_expanded { MENU_SIZE } else { BALL_SIZE } / 2.0) * scale,
    };
    let cx = pos.x as f64 + cur_half;
    let cy = pos.y as f64 + cur_half;

    let new_size = ((if expanded { MENU_SIZE } else { BALL_SIZE }) * scale).round();
    let new_half = new_size / 2.0;
    let mut nx = cx - new_half;
    let mut ny = cy - new_half;

    // 展开时记住原球心；收起时优先恢复（展开被钳制挪位后，球回到原吸附点而非漂移）。
    // 存球心而非窗口左上角：恢复时按目标半边长重新定位，与窗口实际尺寸/DPI 无关
    // （DPI 在展开期间变化时按左上角恢复会让球心漂移半边长差）。
    // 已处于展开态时跳过记录（收回动画期间重复 expand 不覆盖原始吸附点）
    let mut pre = PRE_EXPAND_POS.lock().unwrap_or_else(|e| e.into_inner());
    if expanded {
        if !was_expanded {
            *pre = Some((cx.round() as i32, cy.round() as i32));
        }
    } else if let Some((px, py)) = pre.take() {
        nx = px as f64 - new_half;
        ny = py as f64 - new_half;
    }
    drop(pre);

    if let Ok(Some(mon)) = win.current_monitor() {
        let m = if expanded { 4.0 * scale } else { 0.0 };
        let min_x = mon.position().x as f64 + m;
        let min_y = mon.position().y as f64 + m;
        let max_x = (mon.position().x + mon.size().width as i32) as f64 - new_size - m;
        let max_y = (mon.position().y + mon.size().height as i32) as f64 - new_size - m;
        // 小屏保护：max 可能小于 min（f64::clamp 在 min > max 时 panic）
        let (max_x, max_y) = (max_x.max(min_x), max_y.max(min_y));
        nx = nx.clamp(min_x, max_x);
        ny = ny.clamp(min_y, max_y);
    }

    // 原子应用尺寸+位置（单次 SetWindowPos）：拆成 set_size + set_position 会让窗口
    // 先单向长大再挪回（球心瞬移），WebView2 还要做两次重排——展开卡顿的一部分
    let expect =
        ((if was_expanded { MENU_SIZE } else { BALL_SIZE }) * scale).round() as i32;
    if let Ok(sz) = win.outer_size() {
        let actual = sz.width as i32;
        // 失配修正诊断日志：物理尺寸偏离「逻辑×实时DPI」说明经历过 DPI 失配，
        // 本次调用即自愈（用户反馈「两侧被遮盖」时先查这条日志）
        if (actual - expect).abs() > 1 {
            log::info!(
                "[悬浮球] DPI 失配自愈: 窗口物理 {} → 期望 {} (scale {:.4})",
                actual,
                expect,
                scale
            );
        }
    }
    if let Ok(hwnd) = win.hwnd() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
        };
        unsafe {
            SetWindowPos(
                hwnd.0,
                std::ptr::null_mut(),
                nx.round() as i32,
                ny.round() as i32,
                new_size as i32,
                new_size as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    } else {
        let _ = win.set_size(PhysicalSize::new(new_size as u32, new_size as u32));
        let _ = win.set_position(PhysicalPosition::new(nx.round() as i32, ny.round() as i32));
    }
}

/// 设置变更后的应用：启用则确保窗口存在并联动显隐；停用只隐藏、不销毁窗口。
/// 窗口由启动 init 预创建后常驻——运行时 destroy/rebuild WebView2 与悬浮球窗口
/// 操作交错会卡死整窗（见 init 注释），与 clipboard 浮层「预创建隐藏常驻」同款约束
#[cfg(target_os = "windows")]
fn apply_enabled(app: &AppHandle, enabled: bool) {
    if enabled {
        // 正常路径窗口启动时已预创建；此处 ensure 仅兜底极少见的缺失场景
        if let Err(e) = ensure_window(app) {
            log::warn!("悬浮球窗口创建失败: {}", e);
            return;
        }
        sync_with_main(app);
    } else if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.hide();
        log::info!("悬浮球已停用（窗口隐藏常驻）");
    }
}

// ---------- Tauri 命令 ----------

/// 命令名 = 函数名（Tauri v2 注册规则），前端统一以 `floating_ball_*` 调用，
/// 故函数名带模块前缀，与 src/api/tauri.ts 的 invoke 名一一对应。
#[tauri::command]
pub fn floating_ball_get_state(app: tauri::AppHandle) -> FloatingBallState {
    let cfg = config::load();
    #[cfg(target_os = "windows")]
    let dock = app
        .get_webview_window(LABEL)
        .map(|w| dock_state(&w, cfg.floating_ball_x, cfg.floating_ball_y))
        .unwrap_or_default();
    #[cfg(not(target_os = "windows"))]
    let dock = {
        let _ = &app;
        DockState::default()
    };
    FloatingBallState {
        enabled: cfg.floating_ball_enabled,
        auto_hide: cfg.floating_ball_auto_hide,
        with_main: cfg.floating_ball_with_main,
        buttons: cfg.floating_ball_buttons,
        x: cfg.floating_ball_x,
        y: cfg.floating_ball_y,
        dock,
        ball_size: BALL_SIZE,
        menu_size: MENU_SIZE,
    }
}

/// 保存设置并立即生效（窗口创建/销毁、显隐联动；按钮列表去重 + 钳制上限）。
/// 独立于 save_config：位置等 Rust 侧字段不经过前端快照，避免互相覆盖。
#[tauri::command]
pub fn floating_ball_save_settings(
    app: AppHandle,
    enabled: bool,
    auto_hide: bool,
    with_main: bool,
    buttons: Vec<String>,
) -> Result<(), String> {
    // 去重保序 + 截断上限
    let mut seen = std::collections::HashSet::new();
    let buttons: Vec<String> = buttons
        .into_iter()
        .filter(|b| seen.insert(b.clone()))
        .take(MAX_BUTTONS)
        .collect();

    {
        let _guard = config::lock();
        let mut cfg = config::load();
        cfg.floating_ball_enabled = enabled;
        cfg.floating_ball_auto_hide = auto_hide;
        cfg.floating_ball_with_main = with_main;
        cfg.floating_ball_buttons = buttons;
        config::save(&cfg)?;
    }

    #[cfg(target_os = "windows")]
    apply_enabled(&app, enabled);

    // 通知球窗口重拉状态（按钮集/自动隐藏/停靠边一并刷新）
    #[cfg(target_os = "windows")]
    if enabled {
        use tauri::Emitter;
        let _ = app.emit_to(LABEL, "floating-ball-config-changed", ());
    }

    #[cfg(not(target_os = "windows"))]
    let _ = (&app, auto_hide);

    log::info!(
        "悬浮球设置已保存: enabled={} auto_hide={} with_main={}",
        enabled,
        auto_hide,
        with_main
    );
    Ok(())
}

/// 拖拽结束：球心钳在工作区内 + 可选贴边自动隐藏（球心落到屏边、半隐，
/// 见模块注释）+ 记忆球心到配置。
/// 拖拽只发生在球态，窗口位置 = 球心 - 球态半边长。
/// async：配置读写是文件 IO，必须离开主线程（前端在系统拖动循环结束后调用）。
#[tauri::command]
pub async fn floating_ball_drag_end(app: AppHandle) {
    #[cfg(target_os = "windows")]
    {
        let Some(win) = app.get_webview_window(LABEL) else { return };
        let Ok(pos) = win.outer_position() else { return };
        let scale = window_scale(&win);
        let half = (BALL_SIZE * scale / 2.0).round();
        // 拖拽结束时的球心（球态窗口中心即球心）
        let mut cx = pos.x as f64 + half;
        let mut cy = pos.y as f64 + half;
        if let Ok(Some(mon)) = win.current_monitor() {
            // 以工作区为界（扣除任务栏）：球心落在工作区边缘外会被任务栏盖住
            let wa = mon.work_area();
            let mx = wa.position.x as f64;
            let my = wa.position.y as f64;
            let mr = mx + wa.size.width as f64;
            let mb = my + wa.size.height as f64;
            // 球心钳在显示器内（整个球不出屏，否则拖不回来）
            cx = cx.clamp(mx, mr.max(mx));
            cy = cy.clamp(my, mb.max(my));
            if config::load().floating_ball_auto_hide {
                let trigger = DOCK_TRIGGER * scale;
                let d_left = cx - mx;
                let d_right = mr - cx;
                let d_top = cy - my;
                let d_bottom = mb - cy;
                // 贴边判定：球缘距屏边 < trigger - BALL_R*scale（约一个球半径）
                // → 球心精确落在屏边，只露出半个球体（悬停时前端 CSS 滑出露全）。
                // 两轴独立判定，角上可双侧停靠
                if d_left < trigger && d_left <= d_right {
                    cx = mx;
                } else if d_right < trigger {
                    cx = mr;
                }
                if d_top < trigger && d_top <= d_bottom {
                    cy = my;
                } else if d_bottom < trigger {
                    cy = mb;
                }
            }
        }
        let nx = (cx - half).round() as i32;
        let ny = (cy - half).round() as i32;
        if (nx, ny) != (pos.x, pos.y) {
            let _ = win.set_position(PhysicalPosition::new(nx, ny));
        }
        let _guard = config::lock();
        let mut cfg = config::load();
        cfg.floating_ball_x = Some(cx.round());
        cfg.floating_ball_y = Some(cy.round());
        let _ = config::save(&cfg);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = app;
}

/// 展开/收起环形菜单：以球心为锚切换窗口几何（球态 100 ↔ 菜单态 260，一次原子
/// SetWindowPos）。WebView2 重排滞后帧由前端开合淡出掩盖（见 FloatingBallWindow）。
/// async 与 drag_end 同理（窗口操作离开主线程）。
#[tauri::command]
pub async fn floating_ball_expand(app: AppHandle, expanded: bool) {
    #[cfg(target_os = "windows")]
    if let Some(win) = app.get_webview_window(LABEL) {
        apply_geometry(&win, expanded, None);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (app, expanded);
}

/// 环形菜单/双击动作分发：view:*/act:search/act:note 作用于主窗口（先显示再派发事件）；
/// act:clipboard 直呼剪贴板浮层（不弹主窗）；act:main 双击切换主窗口（开着则收起）
#[tauri::command]
pub fn floating_ball_trigger(app: AppHandle, id: String) {
    if id == "act:clipboard" {
        crate::clipboard::toggle_overlay(&app);
        return;
    }
    if id == "act:main" {
        crate::tray::toggle_main_window(&app);
        return;
    }
    crate::tray::show_window(&app);
    if id != "act:main" {
        use tauri::Emitter;
        let _ = app.emit_to("main", "floating-ball-action", id);
    }
}

/// 右键菜单：托盘同款（tray.rs 统一定义文案，事件 id 带 fb- 前缀）
#[tauri::command]
pub fn floating_ball_context_menu(app: AppHandle) -> Result<(), String> {
    crate::tray::popup_context_menu(&app).map_err(|e| e.to_string())
}

/// 前端失配自检兜底：视口尺寸（CSS clientWidth）偏离期望逻辑尺寸时调用。
/// 携带 clientWidth 让 Rust 反推 WebView2 实际光栅缩放（窗口物理宽 ÷ 视口宽）——
/// 窗口 DPI 上下文过期时 GetDpiForWindow 也会返回旧值，只有视口实测值不会骗人，
/// 据此重设物理尺寸即可把视口拉回期望逻辑尺寸（118% 等自定义缩放下菜单按钮
/// 外圈被裁的根治路径）。与 expand 的区别：不切换态，只把当前态几何拉回正确值。
/// async 与 expand 同理（窗口操作离开主线程）。
#[tauri::command]
pub async fn floating_ball_reapply(app: AppHandle, viewport_w: f64) {
    #[cfg(target_os = "windows")]
    if let Some(win) = app.get_webview_window(LABEL) {
        let override_scale = win.outer_size().ok().and_then(|sz| {
            let s = sz.width as f64 / viewport_w;
            // 合理性区间外的值不可信（视口未就绪/异常），退回 GetDpiForWindow
            (viewport_w > 20.0 && (0.8..=4.0).contains(&s)).then_some(s)
        });
        apply_geometry(&win, is_expanded(&win), override_scale);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (app, viewport_w);
}
