use std::process::Command;

/// 让子进程不弹控制台窗口（链式：`Command::new("x").no_console_window()`）。
///
/// 宿主是 GUI 子系统进程（`main.rs` 的 `windows_subsystem = "windows"`），自身没有控制台；
/// 此时拉起控制台子系统程序（`node.exe` / `cmd` / `powershell` / `netsh` / `reg`）若不带
/// `CREATE_NO_WINDOW`，Windows 会**为子进程新建一个控制台窗口**——表现为「闪一下黑窗」。
///
/// **全工程唯一实现**：`autostart` / `runtime` / `service` / `commands` 都从这里取，
/// 新增子进程调用点直接挂 `.no_console_window()`，不要再抄一份 `creation_flags(0x08000000)`。
pub(crate) trait NoConsoleWindow {
    fn no_console_window(&mut self) -> &mut Self;
}

#[cfg(target_os = "windows")]
impl NoConsoleWindow for Command {
    fn no_console_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x08000000) // CREATE_NO_WINDOW
    }
}

#[cfg(not(target_os = "windows"))]
impl NoConsoleWindow for Command {
    fn no_console_window(&mut self) -> &mut Self {
        self
    }
}

pub fn launch_program(path: &str, args: Option<&str>) -> Result<(), String> {
    let target = std::path::Path::new(path);
    let mut cmd = if target.is_file() {
        let mut c = Command::new(path);
        // 便携软件（如绿色版 exe）依赖同目录资源文件，工作目录设为 exe 所在目录
        if let Some(dir) = target.parent() {
            c.current_dir(dir);
        }
        // 宿主是 GUI 子系统进程（无控制台），直接启动 CLI 工具/bat 时若不指定
        // CREATE_NO_WINDOW，Windows 会为子进程新建控制台窗口（闪黑窗）
        c.no_console_window();
        c
    } else {
        #[cfg(target_os = "windows")]
        let mut c = Command::new("cmd");
        #[cfg(target_os = "windows")]
        {
            // 引号包裹路径，兼容含空格路径；隐藏控制台窗口
            c.arg("/C").arg(format!("\"{}\"", path));
            c.no_console_window();
        }
        #[cfg(not(target_os = "windows"))]
        let mut c = Command::new("sh");
        #[cfg(not(target_os = "windows"))]
        c.arg("-c").arg(path);
        c
    };
    if let Some(args) = args {
        if !args.trim().is_empty() {
            for arg in split_args(args) {
                cmd.arg(arg);
            }
        }
    }
    match cmd.spawn() {
        Ok(_) => Ok(()),
        // Windows 错误 740：程序需要管理员权限，自动请求 UAC 提权
        Err(e) if e.raw_os_error() == Some(740) => {
            log::warn!("程序需要管理员权限，请求 UAC 提权: {}", path);
            launch_elevated(path, args)
        }
        Err(e) => Err(format!("启动程序失败「{}」: {}", path, e)),
    }
}

/// 以管理员权限启动（触发 UAC 提权确认）：PowerShell Start-Process -Verb RunAs
fn launch_elevated(path: &str, args: Option<&str>) -> Result<(), String> {
    let has_args = args.map(|a| !a.trim().is_empty()).unwrap_or(false);
    let script = if has_args {
        "Start-Process -FilePath $env:XHUB_PATH -ArgumentList $env:XHUB_ARGS -Verb RunAs"
    } else {
        "Start-Process -FilePath $env:XHUB_PATH -Verb RunAs"
    };
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
        .env("XHUB_PATH", path);
    cmd.no_console_window();
    if has_args {
        cmd.env("XHUB_ARGS", args.unwrap_or(""));
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("提权启动失败「{}」: {}", path, e))
}

/// 引号感知的参数分割：`--dir "C:\My Apps"` 保持为一个参数
fn split_args(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for c in s.chars() {
        match c {
            '"' => in_quote = !in_quote,
            ' ' | '\t' if !in_quote => {
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

pub fn open_url(url: &str) -> Result<(), String> {
    opener::open(url).map_err(|e| format!("打开链接失败: {}", e))
}

/// 用指定浏览器打开 URL（browser_exe 必须存在；URL 仅放行 http/https，由调用方校验）
pub fn open_with_browser(browser_exe: &str, url: &str) -> Result<(), String> {
    let path = std::path::Path::new(browser_exe);
    if !path.is_file() {
        return Err(format!("浏览器不存在: {}", browser_exe));
    }
    // 浏览器已经在运行：先把它的窗口调度到前台。主流浏览器收到 URL 会复用现有实例、
    // 以新标签页打开，这里再置前一次，避免用户以为「点了没反应」。
    let _ = activate_existing(browser_exe);
    let mut cmd = Command::new(path);
    cmd.arg(url);
    cmd.no_console_window();
    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("启动浏览器失败「{}」: {}", browser_exe, e)),
    }
}

/// 若已有与 `exe_path` 同名的进程在运行，把它的顶层窗口还原并调度到前台，返回 true。
///
/// 用途：速达里点击一个**已经在运行**的应用/浏览器时，不再拉起第二个实例（很多程序不自己
/// 复用实例，会再开一个窗口/进程），而是把已有窗口直接拉出来。
///
/// 只有「目标确实是一个可执行文件、且找得到它的顶层窗口」才返回 true；进程没在跑、只有
/// 后台进程、或窗口全被隐藏时返回 false，调用方应照常启动。非 Windows 平台恒返回 false。
pub fn activate_existing(exe_path: &str) -> bool {
    if !std::path::Path::new(exe_path).is_file() {
        return false;
    }
    match exe_file_name(exe_path) {
        Some(name) => activate_existing_by_name(&name),
        None => false,
    }
}

/// 取可执行文件名（按进程名匹配用；输入可以是完整路径）
fn exe_file_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

#[cfg(target_os = "windows")]
fn activate_existing_by_name(exe_name: &str) -> bool {
    use sysinfo::{ProcessesToUpdate, System};

    // 同名进程可能有一堆（浏览器的渲染/GPU 子进程等）：先收集 PID，再看谁的窗口是主窗口
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let self_pid = std::process::id();
    let mut pids: Vec<u32> = sys
        .processes()
        .iter()
        .filter(|(pid, p)| {
            pid.as_u32() != self_pid && p.name().to_string_lossy().eq_ignore_ascii_case(exe_name)
        })
        .map(|(pid, _)| pid.as_u32())
        .collect();
    pids.sort_unstable();
    pids.dedup();
    if pids.is_empty() {
        return false;
    }
    focus_windows_of(&pids)
}

/// 在给定 PID 集合里找一个顶层窗口并调度到前台：优先可见、非工具窗口；都没有则退回任意非工具窗口
#[cfg(target_os = "windows")]
fn focus_windows_of(pids: &[u32]) -> bool {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetWindowLongW, GetWindowThreadProcessId, IsWindowVisible,
        SetForegroundWindow, ShowWindow, GWL_EXSTYLE, SW_MINIMIZE, SW_RESTORE, WS_EX_TOOLWINDOW,
    };

    struct Ctx {
        pids: Vec<u32>,
        visible: Option<HWND>,
        any: Option<HWND>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam as *mut Ctx);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if !ctx.pids.contains(&pid) {
            return TRUE;
        }
        // 工具窗口（托盘气泡、隐藏的辅助窗）不算主窗口
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW != 0 {
            return TRUE;
        }
        if IsWindowVisible(hwnd) != 0 {
            ctx.visible = Some(hwnd);
            return 0; // 找到可见主窗口即可停（返回 0 终止枚举）
        }
        if ctx.any.is_none() {
            ctx.any = Some(hwnd);
        }
        TRUE
    }

    let mut ctx = Ctx {
        pids: pids.to_vec(),
        visible: None,
        any: None,
    };
    unsafe {
        EnumWindows(Some(enum_proc), &mut ctx as *mut Ctx as LPARAM);
    }
    let Some(hwnd) = ctx.visible.or(ctx.any) else {
        return false;
    };
    unsafe {
        // 最小化的窗口先还原（SW_RESTORE 对非最小化窗口无害）
        ShowWindow(hwnd, SW_RESTORE);
        BringWindowToTop(hwnd);
        if SetForegroundWindow(hwnd) == 0 {
            // 非前台进程调用会被系统拒绝：最小化再还原一次是通行的绕行写法
            ShowWindow(hwnd, SW_MINIMIZE);
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
    true
}

#[cfg(not(target_os = "windows"))]
fn activate_existing_by_name(_exe_name: &str) -> bool {
    false
}

/// 打开外部链接（仅供前端调用的安全命令：只放行 http/https，防止任意 scheme 注入）。
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只能打开 http/https 链接".to_string());
    }
    open_url(&url)
}

/// 打开本地路径：文件用系统默认程序打开，文件夹由资源管理器/文件管理器打开
pub fn open_path(path: &str) -> Result<(), String> {
    let target = std::path::Path::new(path);
    if target.is_dir() {
        #[cfg(target_os = "windows")]
        {
            Command::new("explorer")
                .arg(path)
                .spawn()
                .map_err(|e| format!("打开文件夹失败: {}", e))?;
            return Ok(());
        }
        #[cfg(not(target_os = "windows"))]
        {
            opener::open(path).map_err(|e| format!("打开文件夹失败: {}", e))?;
            return Ok(());
        }
    }
    opener::open(path).map_err(|e| format!("打开路径失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_nonexistent_program_returns_error() {
        // Windows 上经 cmd /C 启动不存在路径时 cmd 进程本身可成功 spawn，
        // 因此只对非 Windows 平台断言失败；Windows 断言不 panic 即可。
        #[cfg(not(target_os = "windows"))]
        {
            let result = launch_program("/nonexistent/path/xyz", None);
            assert!(result.is_err());
        }
        #[cfg(target_os = "windows")]
        {
            let _ = launch_program("/nonexistent/path/xyz", None);
        }
    }

    #[test]
    fn exe_file_name_extracts_basename() {
        assert_eq!(
            exe_file_name(r"C:\Program Files\Google\Chrome\Application\chrome.exe").as_deref(),
            Some("chrome.exe")
        );
        assert_eq!(exe_file_name("/usr/bin/firefox").as_deref(), Some("firefox"));
        assert_eq!(exe_file_name("chrome.exe").as_deref(), Some("chrome.exe"));
        assert_eq!(exe_file_name(""), None);
    }

    #[test]
    fn activate_existing_ignores_non_file_targets() {
        // 目标不是文件（命令行 / URL）时不该去匹配进程，恒 false
        assert!(!activate_existing("not-a-real-file-xyz"));
        assert!(!activate_existing("https://example.com"));
    }
}
