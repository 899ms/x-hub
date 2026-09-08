//! 自定义右下角通知弹窗（跨 Win10 / Win11 一致，替代系统 WinRT Toast）。
//!
//! 为什么不用系统通知：Windows 10 对「非安装包 / 便携版」应用有额外硬门槛——AUMID 必须能在
//! 开始菜单里找到对应 .lnk，否则 `CreateToastNotifier` 判定应用不具备通知能力，Toast 被静默丢弃
//! （表现就是 Win11 正常、Win10 完全无弹窗）。与其去维护快捷方式，不如自绘一个右下角小窗，
//! 两个版本表现一致、样式可控。
//!
//! 架构：一个透明、置顶、跳过任务栏、不抢焦点（WS_EX_NOACTIVATE + SW_SHOWNA）的独立 WebView 窗
//! （label = `notice`，前端 App.vue 路由到 NoticeOverlay 渲染卡片堆叠）。后端只做：建窗/隐藏常驻、
//! 把一条通知 emit 给前端、以及按前端上报的内容高度把窗口锚定到「当前显示器工作区」右下角（避让
//! 任务栏）。卡片的入/出场动画、逐条自动淡出计时、点击关闭都由前端负责，后端无状态。
//!
//! 启动时 `init` 预创建隐藏窗口（同剪贴板浮层/悬浮球的约定：运行时现场建 WebView2 窗口易与并发
//! 窗口操作交错导致整窗未响应）。推送时后端**先**把窗口定位+无激活显示，再 emit 内容——避免依赖
//! 隐藏态 WebView2 是否及时处理 IPC 事件的不确定性。

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// 通知窗 label（App.vue 按此路由渲染 NoticeOverlay）
pub const NOTICE_LABEL: &str = "notice";
/// 通知窗宽度（逻辑 px）
const NOTICE_WIDTH: f64 = 360.0;
/// 距工作区右 / 下边留白（逻辑 px）：8 + 前端 stack 内衬 8 = 卡片离任务栏/屏幕右沿各 16px
const NOTICE_MARGIN: f64 = 8.0;
/// 窗口默认高度（首帧定位用；随后前端按实际内容高度回调 notice_layout 校正）
const NOTICE_DEFAULT_HEIGHT: f64 = 104.0;

/// 启动时预创建通知窗（隐藏常驻）。
pub fn init(app: &AppHandle) {
    match ensure_window(app) {
        Ok(_) => log::info!("通知窗已预创建（隐藏常驻）"),
        Err(e) => log::warn!("通知窗预创建失败: {e}"),
    }
}

/// 推送一条通知：确保窗口存在 → 先定位并无激活显示 → emit `notice-new` 给前端渲染。
/// `kind` 供前端选图标/配色（"countdown" | "todo" | "info" ...）。
pub fn show_notice(app: &AppHandle, kind: &str, title: &str, body: &str) {
    let win = match ensure_window(app) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("通知窗不可用，丢弃一条通知（{title}）: {e}");
            return;
        }
    };
    // 先定位 + 显示：不依赖隐藏 WebView2 处理事件的时序，卡片随后由前端 layout 精确校正高度
    anchor_default_and_show(&win);
    let payload = serde_json::json!({ "kind": kind, "title": title, "body": body });
    if let Err(e) = app.emit_to(NOTICE_LABEL, "notice-new", &payload) {
        log::warn!("通知事件投递失败: {e}");
    }
    log::info!("已推送右下角通知: [{kind}] {title}");
}

fn ensure_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(w) = app.get_webview_window(NOTICE_LABEL) {
        return Ok(w);
    }
    build_window(app)
}

fn build_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let mut builder = WebviewWindowBuilder::new(app, NOTICE_LABEL, WebviewUrl::App("index.html".into()))
        .title("通知")
        .inner_size(NOTICE_WIDTH, NOTICE_DEFAULT_HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .additional_browser_args(crate::ADDITIONAL_BROWSER_ARGS);

    // 透明窗口在 Windows 上开系统阴影会渲染成黑描边；卡片自带 CSS 阴影，关掉 OS 阴影（同倒计时/剪贴板浮窗）
    #[cfg(target_os = "windows")]
    {
        builder = builder.shadow(false);
    }

    builder.build().map_err(|e| e.to_string())
}

/// 用默认高度把窗口锚到右下角并无激活显示（供推送首帧；精确高度随后由 notice_layout 校正）。
fn anchor_default_and_show(win: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        apply_size(win, NOTICE_WIDTH, NOTICE_DEFAULT_HEIGHT);
        anchor_bottom_right(win, NOTICE_WIDTH, NOTICE_DEFAULT_HEIGHT);
        show_no_activate(win);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = win.set_resizable(true);
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            NOTICE_WIDTH,
            NOTICE_DEFAULT_HEIGHT,
        )));
        let _ = win.set_resizable(false);
        simple_bottom_right(win, NOTICE_WIDTH, NOTICE_DEFAULT_HEIGHT);
        let _ = win.show();
    }
}

/// 前端上报实际内容高度 → 设尺寸并重新锚定右下角（保持底边贴住工作区下沿，卡片向上生长）。
#[tauri::command]
pub fn notice_layout(app: AppHandle, height: f64) -> Result<(), String> {
    let win = app
        .get_webview_window(NOTICE_LABEL)
        .ok_or_else(|| "通知窗不存在".to_string())?;
    let h = height.clamp(1.0, 1600.0);
    #[cfg(target_os = "windows")]
    {
        apply_size(&win, NOTICE_WIDTH, h);
        anchor_bottom_right(&win, NOTICE_WIDTH, h);
        show_no_activate(&win);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            NOTICE_WIDTH,
            h,
        )));
        simple_bottom_right(&win, NOTICE_WIDTH, h);
        let _ = win.show();
    }
    Ok(())
}

/// 队列为空时前端收起窗口（仅隐藏，窗口常驻复用）。
#[tauri::command]
pub fn notice_dismiss_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(NOTICE_LABEL) {
        hide_window(&win);
    }
    Ok(())
}

// ---------- Windows：工作区右下角 + 不抢焦点显示 ----------

#[cfg(target_os = "windows")]
fn apply_size(win: &WebviewWindow, width_logical: f64, height_logical: f64) {
    // resizable(false) 会把 min/max 锁死在当前尺寸，后续 set_size 被 WM_GETMINMAXINFO 钳制而静默失效
    // （表现为窗口一直保持默认 104 高、卡片下方多出一条透明空隙）——临时解锁 → 改尺寸 → 再锁回。
    let _ = win.set_resizable(true);
    let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
        width_logical,
        height_logical,
    )));
    let _ = win.set_resizable(false);
}

/// 计算并设置物理像素位置：光标所在显示器的工作区（rcWork，避让任务栏）右下角内缩 margin。
/// 取光标所在屏（而非窗口所在屏）：通知应出现在用户正在操作的那块屏上（多屏体验一致）。
#[cfg(target_os = "windows")]
fn anchor_bottom_right(win: &WebviewWindow, width_logical: f64, height_logical: f64) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    };
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let Ok(hwnd) = win.hwnd() else {
        return;
    };
    unsafe {
        let dpi = GetDpiForWindow(hwnd.0);
        let scale = if dpi > 0 { dpi as f64 / 96.0 } else { 1.0 };
        let w = (width_logical * scale).round() as i32;
        let h = (height_logical * scale).round() as i32;
        let m = (NOTICE_MARGIN * scale).round() as i32;

        // 光标所在显示器；取不到光标（异常）则退回窗口所在显示器
        let monitor = {
            let mut pt: POINT = std::mem::zeroed();
            if GetCursorPos(&mut pt) != 0 {
                MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
            } else {
                windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(
                    hwnd.0,
                    MONITOR_DEFAULTTONEAREST,
                )
            }
        };
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return;
        }
        let work = info.rcWork;
        // 工作区比窗口还小的极端情况用 max 兜底，避免负坐标跑出屏幕
        let x = (work.right - w - m).max(work.left);
        let y = (work.bottom - h - m).max(work.top);
        let _ = win.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)));
    }
}

/// 无激活显示：加 WS_EX_NOACTIVATE 后 SW_SHOWNA——通知不该抢走用户当前输入焦点。
#[cfg(target_os = "windows")]
fn show_no_activate(win: &WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_SHOWNA, WS_EX_NOACTIVATE,
    };
    if let Ok(hwnd) = win.hwnd() {
        unsafe {
            let ex = GetWindowLongPtrW(hwnd.0, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd.0, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE as isize);
            ShowWindow(hwnd.0, SW_SHOWNA);
        }
        return;
    }
    let _ = win.show();
}

#[cfg(target_os = "windows")]
fn hide_window(win: &WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if let Ok(hwnd) = win.hwnd() {
        unsafe {
            ShowWindow(hwnd.0, SW_HIDE);
        }
        return;
    }
    let _ = win.hide();
}

// ---------- 非 Windows 兜底：用 Tauri 显示器逻辑（无工作区，仅整屏右下） ----------

#[cfg(not(target_os = "windows"))]
fn simple_bottom_right(win: &WebviewWindow, width_logical: f64, height_logical: f64) {
    let scale = win.scale_factor().unwrap_or(1.0);
    let monitor = win
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| win.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let size = m.size(); // 物理 px
    let origin = m.position();
    let w = (width_logical * scale).round() as i32;
    let h = (height_logical * scale).round() as i32;
    let m2 = (NOTICE_MARGIN * scale).round() as i32;
    let x = origin.x + size.width as i32 - w - m2;
    let y = origin.y + size.height as i32 - h - m2;
    let _ = win.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(x, y)));
}
