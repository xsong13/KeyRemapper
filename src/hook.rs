//! 低层键盘钩子与组合键状态机。
//!
//! 关键约束：LowLevelKeyboardProc 在超时后会被 Windows 静默卸载，
//! 因此回调内只做「内存查表 + 一次 SendInput」，不读文件、不弹窗、不等待锁。
//! 规则表用 RwLock + try_read：写锁被占用时宁可放行，也不阻塞输入。
//! 按键状态放 thread_local：钩子始终在安装它的专用线程上执行，无需加锁。

use crate::config::{Config, RuleMap};
use crate::keys::{self, KeyId, K_ESC};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Mutex, OnceLock, RwLock};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 注入事件标记。比只检查 LLKHF_INJECTED 更精确：不会误放行其他程序的注入。
const MAGIC: usize = 0x4B52_4D41_5050_4552;

pub const WM_APP_CAPTURE: u32 = WM_APP + 2;
pub const WM_APP_CAPTURE_CANCEL: u32 = WM_APP + 3;
pub const CAP_TRIGGER: u8 = 1;
pub const CAP_OUTPUT: u8 = 2;

struct Engine {
    rules: RuleMap,
    enabled: bool,
}

static ENGINE: OnceLock<RwLock<Engine>> = OnceLock::new();
static SEND_FAILURES: AtomicU32 = AtomicU32::new(0);

pub fn init_engine(cfg: &Config) {
    let _ = ENGINE.set(RwLock::new(Engine {
        rules: cfg.to_map(),
        enabled: cfg.enabled,
    }));
}

pub fn apply(cfg: &Config) {
    if let Some(lock) = ENGINE.get() {
        if let Ok(mut e) = lock.write() {
            e.rules = cfg.to_map();
            e.enabled = cfg.enabled;
        }
    }
}

/// 取出并清零 SendInput 失败计数，供 UI 提示（回调内不做任何 IO）。
pub fn take_failures() -> u32 {
    SEND_FAILURES.swap(0, Ordering::Relaxed)
}

struct Capture {
    stage: u8,
    hwnd: isize,
    mods: Vec<KeyId>,
    held: Vec<KeyId>,
    result: Vec<KeyId>,
}

static CAPTURE: Mutex<Capture> = Mutex::new(Capture {
    stage: 0,
    hwnd: 0,
    mods: Vec::new(),
    held: Vec::new(),
    result: Vec::new(),
});

pub fn begin_capture(stage: u8, hwnd: isize) {
    if let Ok(mut cap) = CAPTURE.lock() {
        cap.stage = stage;
        cap.hwnd = hwnd;
        cap.mods.clear();
        cap.result.clear();
        // 清掉上一轮未松开的键，避免残留状态把后续按键全部吞掉。
        cap.held.clear();
    }
}

pub fn end_capture() {
    if let Ok(mut cap) = CAPTURE.lock() {
        cap.stage = 0;
        cap.mods.clear();
        cap.held.clear();
    }
}

pub fn take_capture_result() -> Vec<KeyId> {
    CAPTURE
        .lock()
        .map(|mut c| std::mem::take(&mut c.result))
        .unwrap_or_default()
}

#[derive(Default)]
struct KeyState {
    /// 当前真实按下的修饰键，保留按下顺序以便原样恢复。
    pressed_mod_keys: Vec<KeyId>,
    /// 被吞掉 keydown 的主键，用于吞掉对应的孤立 keyup。
    suppressed: HashSet<KeyId>,
}

thread_local! {
    static STATE: RefCell<KeyState> = RefCell::new(KeyState::default());
}

/// 启动专用钩子线程。返回 (句柄, 线程 ID)，线程 ID 用于退出时投递 WM_QUIT。
pub fn spawn() -> io::Result<(std::thread::JoinHandle<()>, u32)> {
    let (tx, rx) = mpsc::channel::<Result<u32, i32>>();
    let handle = std::thread::spawn(move || unsafe {
        let hinst = GetModuleHandleW(ptr::null());
        let hhook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hinst, 0);
        if hhook.is_null() {
            // 错误码必须在安装失败的同一线程上读取。
            let _ = tx.send(Err(GetLastError() as i32));
            return;
        }
        // 先创建本线程消息队列，再回报线程 ID：
        // 否则调用方过早 PostThreadMessageW(WM_QUIT) 会因队列尚未建立而丢失。
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_NOREMOVE);
        let _ = tx.send(Ok(GetCurrentThreadId()));

        // WH_KEYBOARD_LL 的回调由安装线程的消息循环派发；此线程无窗口，无需 DispatchMessage。
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            // 空循环体：回调在 GetMessageW 内部被调用。
        }
        UnhookWindowsHookEx(hhook);
    });
    match rx.recv() {
        Ok(Ok(tid)) => Ok((handle, tid)),
        Ok(Err(code)) => Err(io::Error::from_raw_os_error(code)),
        Err(_) => Err(io::Error::other("钩子线程启动失败")),
    }
}

pub fn stop(thread_id: u32) {
    unsafe {
        PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
    }
}

unsafe fn pass(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code != HC_ACTION as i32 {
        return pass(code, wparam, lparam);
    }
    let kb = &*(lparam as *const KBDLLHOOKSTRUCT);

    // 自己注入的事件（以及任何注入事件）直接放行，且不更新内部状态，避免回环。
    if kb.dwExtraInfo == MAGIC || kb.flags & LLKHF_INJECTED != 0 {
        return pass(code, wparam, lparam);
    }

    let msg = wparam as u32;
    if msg != WM_KEYDOWN && msg != WM_KEYUP && msg != WM_SYSKEYDOWN && msg != WM_SYSKEYUP {
        return pass(code, wparam, lparam);
    }
    let is_up = kb.flags & LLKHF_UP != 0;
    let id: KeyId = (kb.scanCode as u16 & 0xFF)
        | if kb.flags & LLKHF_EXTENDED != 0 { 0x100 } else { 0 };

    // 捕获模式：吞掉全部按键直到所有键松开，避免残留按键流向前台程序。
    if let Ok(mut cap) = CAPTURE.try_lock() {
        if cap.stage != 0 || !cap.held.is_empty() {
            return handle_capture(&mut cap, id, is_up);
        }
    }

    STATE.with(|cell| {
        let mut st = cell.borrow_mut();
        if is_up {
            if st.suppressed.remove(&id) {
                return 1;
            }
            st.pressed_mod_keys.retain(|k| *k != id);
            return pass(code, wparam, lparam);
        }

        if keys::is_modifier(id) {
            if !st.pressed_mod_keys.contains(&id) {
                st.pressed_mod_keys.push(id);
            }
            return pass(code, wparam, lparam);
        }

        let mods = keys::mods_of(&st.pressed_mod_keys);
        let output = ENGINE.get().and_then(|e| e.try_read().ok()).and_then(|e| {
            if e.enabled {
                e.rules.get(&(mods, id)).cloned()
            } else {
                None
            }
        });
        match output {
            Some(seq) => {
                st.suppressed.insert(id);
                execute(&st, &seq);
                1
            }
            None => pass(code, wparam, lparam),
        }
    })
}

fn handle_capture(cap: &mut Capture, id: KeyId, is_up: bool) -> LRESULT {
    if is_up {
        cap.held.retain(|k| *k != id);
        cap.mods.retain(|k| *k != id);
        return 1;
    }
    if !cap.held.contains(&id) {
        cap.held.push(id);
    }
    if cap.stage == 0 {
        return 1; // 等待按键全部松开的排空阶段
    }
    if id == K_ESC {
        cap.stage = 0;
        cap.mods.clear();
        cap.result.clear();
        unsafe {
            PostMessageW(cap.hwnd as HWND, WM_APP_CAPTURE_CANCEL, 0, 0);
        }
        return 1;
    }
    if keys::is_modifier(id) {
        if !cap.mods.contains(&id) {
            cap.mods.push(id);
        }
        return 1;
    }

    let stage = cap.stage;
    let mut combo = cap.mods.clone();
    combo.push(id);
    cap.result = combo;
    cap.mods.clear();
    cap.stage = 0;
    unsafe {
        PostMessageW(cap.hwnd as HWND, WM_APP_CAPTURE, stage as WPARAM, 0);
    }
    1
}

fn make_input(id: KeyId, up: bool) -> INPUT {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_KEYBOARD;
    let mut flags = KEYEVENTF_SCANCODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if id & 0x100 != 0 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    input.Anonymous.ki = KEYBDINPUT {
        wVk: 0,
        wScan: id & 0xFF,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: MAGIC,
    };
    input
}

/// 命中规则后的一次性注入序列。
///
/// 用户按下 Ctrl 时系统已把 Ctrl down 发给前台程序；若不处理，
/// 映射 Ctrl+J -> F5 会被目标程序看成 Ctrl+F5。因此先抬起真实修饰键，
/// 注入目标序列，再按原顺序恢复修饰键，保证用户手仍按着时状态一致。
unsafe fn execute(st: &KeyState, output: &[KeyId]) {
    let mut events: Vec<INPUT> =
        Vec::with_capacity(st.pressed_mod_keys.len() * 2 + output.len() * 2);
    for &k in &st.pressed_mod_keys {
        events.push(make_input(k, true));
    }
    for &k in output {
        events.push(make_input(k, false));
    }
    for &k in output.iter().rev() {
        events.push(make_input(k, true));
    }
    for &k in &st.pressed_mod_keys {
        events.push(make_input(k, false));
    }
    if events.is_empty() {
        return;
    }
    let sent = SendInput(
        events.len() as u32,
        events.as_ptr(),
        std::mem::size_of::<INPUT>() as i32,
    );
    if sent != events.len() as u32 {
        SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
}
