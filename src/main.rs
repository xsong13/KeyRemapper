#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod config;
mod hook;
mod keys;
mod ui;
mod util;

use crate::util::wide;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_APP_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const TRAY_GUIDE: &str = "双击任务栏右下角的橙色图标即可设置按键映射。图标也可能在隐藏图标菜单（^）中。";

const M_TOGGLE: usize = 1;
const M_EDIT: usize = 2;
const M_AUTOSTART: usize = 3;
const M_EXIT: usize = 4;

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);
static SMOKE: AtomicBool = AtomicBool::new(false);
static WARNINGS: OnceLock<Vec<String>> = OnceLock::new();
static CLASS_NAME: OnceLock<Vec<u16>> = OnceLock::new();

fn main() {
    // --smoke 仅用于验证钩子与托盘的初始化路径，不进入消息循环。
    SMOKE.store(std::env::args().any(|a| a == "--smoke"), Ordering::Relaxed);
    unsafe {
        // 单实例：避免两个钩子同时拦截同一按键。
        let mutex_name = wide("Local\\KeyRemapper_SingleInstance");
        let _mutex = CreateMutexW(ptr::null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            alert(&format!("KeyRemapper 已在运行。\n{TRAY_GUIDE}"));
            return;
        }

        let (cfg, warnings) = config::Config::load();
        let _ = WARNINGS.set(warnings.clone());
        config::init(cfg.clone());
        hook::init_engine(&cfg);

        let hook_thread = match hook::spawn() {
            Ok((handle, tid)) => {
                HOOK_TID.store(tid, Ordering::Relaxed);
                Some(handle)
            }
            Err(e) => {
                alert(&format!("安装键盘钩子失败：{e}"));
                return;
            }
        };

        let hinst = GetModuleHandleW(ptr::null());
        let class = CLASS_NAME.get_or_init(|| wide("KeyRemapperMain"));
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(main_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: LoadIconW(ptr::null_mut(), IDI_APPLICATION),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let title = wide("KeyRemapper");
        // 隐藏窗口：只用于接收托盘回调消息，从不 ShowWindow。
        let hwnd = CreateWindowExW(
            0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW,
            0, 0, 0, 0,
            ptr::null_mut(), ptr::null_mut(), hinst, ptr::null(),
        );
        if hwnd.is_null() {
            alert("创建主窗口失败。");
            return;
        }
        MAIN_HWND.store(hwnd as isize, Ordering::Relaxed);

        if !add_tray(hwnd) {
            alert("创建托盘图标失败。");
            return;
        }

        if SMOKE.load(Ordering::Relaxed) {
            remove_tray(hwnd);
            hook::stop(HOOK_TID.load(Ordering::Relaxed));
            if let Some(handle) = hook_thread {
                let _ = handle.join();
            }
            eprintln!("smoke ok: hook installed, tray created, config path ready");
            return;
        }

        if !warnings.is_empty() {
            alert(&format!("配置文件有问题：\n{}", warnings.join("\n")));
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // 消息循环退出后停止钩子线程，确保卸载钩子再结束进程。
        hook::stop(HOOK_TID.load(Ordering::Relaxed));
        if let Some(handle) = hook_thread {
            let _ = handle.join();
        }
    }
}

unsafe fn alert(text: &str) {
    if SMOKE.load(Ordering::Relaxed) {
        eprintln!("[KeyRemapper] {text}");
        std::process::exit(1);
    }
    let s = wide(text);
    let caption = wide("KeyRemapper");
    MessageBoxW(ptr::null_mut(), s.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONINFORMATION);
}

unsafe fn add_tray(hwnd: HWND) -> bool {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_APP_TRAY;
    let image = include_bytes!("tray.png");
    nid.hIcon = CreateIconFromResourceEx(
        image.as_ptr(), image.len() as u32, 1, 0x00030000,
        GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR,
    );
    if nid.hIcon.is_null() {
        return false;
    }
    let tip = wide("KeyRemapper - 按键重映射");
    for (i, c) in tip.iter().enumerate().take(nid.szTip.len()) {
        nid.szTip[i] = *c;
    }
    if !SMOKE.load(Ordering::Relaxed) {
        nid.uFlags |= NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
        let title = wide("KeyRemapper 已启动");
        nid.szInfoTitle[..title.len()].copy_from_slice(&title);
        let guide = wide(TRAY_GUIDE);
        nid.szInfo[..guide.len()].copy_from_slice(&guide);
    }
    let added = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
    DestroyIcon(nid.hIcon);
    added
}

unsafe fn remove_tray(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn show_tray_menu(hwnd: HWND) -> usize {
    let menu = CreatePopupMenu();
    let enabled = config::snapshot().enabled;
    let toggle = wide(if enabled { "暂停映射" } else { "启用映射" });
    let edit = wide("编辑映射...");
    let autostart = wide("开机自启");
    let exit = wide("退出");
    AppendMenuW(menu, MF_STRING, M_TOGGLE, toggle.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(menu, MF_STRING, M_EDIT, edit.as_ptr());
    let auto_flags = if autostart::is_enabled() { MF_STRING | MF_CHECKED } else { MF_STRING };
    AppendMenuW(menu, auto_flags, M_AUTOSTART, autostart.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(menu, MF_STRING, M_EXIT, exit.as_ptr());

    let mut pt: POINT = std::mem::zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
        pt.x, pt.y, 0, hwnd, ptr::null(),
    );
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    if cmd > 0 { cmd as usize } else { 0 }
}

unsafe fn handle_command(hwnd: HWND, cmd: usize) {
    match cmd {
        M_TOGGLE => {
            let next = !config::snapshot().enabled;
            if let Err(e) = config::set_enabled(next) {
                alert(&format!("保存设置失败：{e}"));
            }
        }
        M_EDIT => {
            ui::open(WARNINGS.get().map(Vec::as_slice).unwrap_or(&[]));
        }
        M_AUTOSTART => {
            let next = !autostart::is_enabled();
            if let Err(e) = autostart::set_enabled(next) {
                alert(&format!("设置开机自启失败：{e}"));
            }
        }
        M_EXIT => {
            ui::close();
            hook::end_capture();
            remove_tray(hwnd);
            PostQuitMessage(0);
        }
        _ => {}
    }
}

unsafe extern "system" fn main_wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_TRAY => {
            let mouse = lp as u32;
            if mouse == WM_RBUTTONUP || mouse == WM_CONTEXTMENU {
                let cmd = show_tray_menu(hwnd);
                if cmd != 0 {
                    handle_command(hwnd, cmd);
                }
            } else if mouse == WM_LBUTTONDBLCLK {
                handle_command(hwnd, M_EDIT);
            }
            0
        }
        WM_DESTROY => {
            remove_tray(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
