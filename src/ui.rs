//! 设置窗口：列出规则、添加/删除、按键捕获。
//!
//! 只用 user32 标准控件（ListBox / Button / Static），不引入 common-controls
//! 或任何 UI 框架，保持单文件、零运行时依赖。

use crate::config;
use crate::hook::{self, CAP_OUTPUT, CAP_TRIGGER, WM_APP_CAPTURE, WM_APP_CAPTURE_CANCEL};
use crate::keys::{self, KeyId};
use crate::util::wide;
use std::cell::RefCell;
use std::ptr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Once, OnceLock};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_LIST: u16 = 1001;
const ID_ADD: u16 = 1002;
const ID_DEL: u16 = 1003;
const ID_HINT: u16 = 1004;
const ID_CLOSE: u16 = 1005;

const LB_ADDSTRING: u32 = 0x0180;
const LB_DELETESTRING: u32 = 0x0182;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_GETCURSEL: u32 = 0x0188;

const CHILD: u32 = 0x4000_0000;
const VISIBLE: u32 = 0x1000_0000;
const BORDER: u32 = 0x0080_0000;
const VSCROLL: u32 = 0x0020_0000;
const LBS_NOTIFY_BIT: u32 = 0x0001;

static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);
static CLASS_ONCE: Once = Once::new();
static CLASS_NAME: OnceLock<Vec<u16>> = OnceLock::new();

struct UiState {
    hwnd: isize,
    list: isize,
    hint: isize,
    pending_trigger: Option<Vec<KeyId>>,
}

thread_local! {
    static UI: RefCell<Option<UiState>> = const { RefCell::new(None) };
}

pub fn is_open() -> bool {
    SETTINGS_HWND.load(Ordering::Relaxed) != 0
}

pub fn bring_to_front() {
    let hwnd = SETTINGS_HWND.load(Ordering::Relaxed) as HWND;
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}

pub fn close() {
    let hwnd = SETTINGS_HWND.load(Ordering::Relaxed);
    if hwnd != 0 {
        unsafe {
            PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
        }
    }
}

pub fn open(warnings: &[String]) {
    if is_open() {
        bring_to_front();
        return;
    }
    register_class();
    unsafe {
        let hinst = GetModuleHandleW(ptr::null());
        let class = CLASS_NAME.get().unwrap();
        let title = wide("KeyRemapper - 按键映射");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            540,
            460,
            ptr::null_mut(),
            ptr::null_mut(),
            hinst,
            ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        SETTINGS_HWND.store(hwnd as isize, Ordering::Relaxed);

        let hint = create_ctrl(hinst, hwnd, "STATIC", ID_HINT, "", 16, 12, 490, 42, CHILD | VISIBLE);
        let list = create_ctrl(
            hinst, hwnd, "LISTBOX", ID_LIST, "", 16, 60, 490, 290,
            CHILD | VISIBLE | BORDER | VSCROLL | LBS_NOTIFY_BIT,
        );
        create_ctrl(hinst, hwnd, "BUTTON", ID_ADD, "添加映射", 16, 362, 110, 30,
            CHILD | VISIBLE | (BS_PUSHBUTTON as u32));
        create_ctrl(hinst, hwnd, "BUTTON", ID_DEL, "删除选中", 136, 362, 110, 30,
            CHILD | VISIBLE | (BS_PUSHBUTTON as u32));
        create_ctrl(hinst, hwnd, "BUTTON", ID_CLOSE, "关闭", 396, 362, 110, 30,
            CHILD | VISIBLE | (BS_PUSHBUTTON as u32));

        UI.with(|cell| {
            *cell.borrow_mut() = Some(UiState {
                hwnd: hwnd as isize,
                list: list as isize,
                hint: hint as isize,
                pending_trigger: None,
            });
        });

        refresh_list(list as isize);
        let failures = hook::take_failures();
        let text = if failures > 0 {
            format!("有 {failures} 次注入失败（目标窗口可能是管理员权限）。规则：点击「添加映射」后按两次组合键。")
        } else if !warnings.is_empty() {
            format!("配置提示：{}。规则：点击「添加映射」后按两次组合键。", warnings[0])
        } else {
            "点击「添加映射」，先按触发组合键，再按目标组合键；Esc 取消。".to_string()
        };
        set_hint(hint as isize, &text);

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

unsafe fn create_ctrl(
    hinst: HINSTANCE,
    parent: HWND,
    class: &str,
    id: u16,
    text: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    style: u32,
) -> HWND {
    let cls = wide(class);
    let label = wide(text);
    CreateWindowExW(
        0,
        cls.as_ptr(),
        label.as_ptr(),
        style,
        x, y, w, h,
        parent,
        id as HMENU,
        hinst,
        ptr::null(),
    )
}

fn register_class() {
    CLASS_ONCE.call_once(|| unsafe {
        let name = CLASS_NAME.get_or_init(|| wide("KeyRemapperSettings"));
        let hinst = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: LoadIconW(ptr::null_mut(), IDI_APPLICATION),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: name.as_ptr(),
        };
        RegisterClassW(&wc);
    });
}

fn set_hint(hwnd: isize, text: &str) {
    let s = wide(text);
    unsafe {
        SetWindowTextW(hwnd as HWND, s.as_ptr());
    }
}

fn refresh_list(list: isize) {
    let cfg = config::snapshot();
    unsafe {
        SendMessageW(list as HWND, LB_RESETCONTENT, 0, 0);
        for r in &cfg.rules {
            let s = wide(&cfg.rule_text(r));
            SendMessageW(list as HWND, LB_ADDSTRING, 0, s.as_ptr() as LPARAM);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    UI.with(|cell| {
        // 控件可能同步回送 WM_CTLCOLOR/WM_COMMAND 造成窗口过程重入；
        // 用 try_borrow_mut 防御，重入消息交给默认处理而不是 panic。
        let Ok(mut opt) = cell.try_borrow_mut() else {
            if msg == WM_DESTROY {
                hook::end_capture();
                SETTINGS_HWND.store(0, Ordering::Relaxed);
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        if msg == WM_DESTROY {
            hook::end_capture();
            SETTINGS_HWND.store(0, Ordering::Relaxed);
            *opt = None;
            return 0;
        }
        let Some(st) = opt.as_mut() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        let (list, hint, self_hwnd) = (st.list, st.hint, st.hwnd);

        match msg {
            WM_COMMAND => {
                let id = (wp & 0xFFFF) as u16;
                match id {
                    ID_ADD => {
                        st.pending_trigger = None;
                        hook::begin_capture(CAP_TRIGGER, self_hwnd);
                        set_hint(hint, "请按下「触发组合键」（如 Ctrl+J），Esc 取消");
                    }
                    ID_DEL => {
                        let sel = SendMessageW(list as HWND, LB_GETCURSEL, 0, 0);
                        if sel >= 0 {
                            SendMessageW(list as HWND, LB_DELETESTRING, sel as WPARAM, 0);
                            match config::remove_rule(sel as usize) {
                                Ok(()) => set_hint(hint, "已删除规则并保存。"),
                                Err(e) => set_hint(hint, &e),
                            }
                            refresh_list(list);
                        } else {
                            set_hint(hint, "请先在列表中选择一条规则。");
                        }
                    }
                    ID_CLOSE => {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                    _ => {}
                }
                0
            }
            WM_APP_CAPTURE => {
                let combo = hook::take_capture_result();
                let stage = wp as u8;
                if stage == CAP_TRIGGER {
                    st.pending_trigger = Some(combo);
                    hook::begin_capture(CAP_OUTPUT, self_hwnd);
                    set_hint(hint, "已记录触发键。请按下「目标组合键」（如 F5），Esc 取消");
                } else if stage == CAP_OUTPUT {
                    match st.pending_trigger.take() {
                        Some(trigger) => match config::add_rule(&trigger, &combo) {
                            Ok(()) => {
                                refresh_list(list);
                                let from = keys::format_combo(&trigger);
                                let to = keys::format_combo(&combo);
                                set_hint(hint, &format!("已添加：{from} -> {to}，已保存并生效。"));
                            }
                            Err(e) => set_hint(hint, &e),
                        },
                        None => {
                            hook::end_capture();
                            set_hint(hint, "触发组合已丢失，请重新添加。");
                        }
                    }
                }
                0
            }
            WM_APP_CAPTURE_CANCEL => {
                st.pending_trigger = None;
                hook::end_capture();
                set_hint(hint, "已取消。点击「添加映射」重新开始。");
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    })
}
