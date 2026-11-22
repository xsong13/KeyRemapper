# KeyRemapper

轻量级 Windows 全局按键重映射工具。支持组合键、开机自启，单文件约 0.2 MB，零额外运行时依赖。

## 一个典型场景

想截个图，却得先打开 QQ 或微信；或者总是记不住 Windows 自带的截图快捷键，系统又不提供修改入口——如果你也遇到过，可以用 KeyRemapper 把它换成一个顺手的键：

```text
F1 = Win+Shift+S
```

`Win+Shift+S` 是 Windows 10/11 默认的「截图工具」框选截图快捷键，体验接近 QQ / 微信截图，但组合键长、无法自定义。映射后单按 `F1` 即可呼出；想要全屏截图也可以写成 `F1 = PrintScreen`，当前窗口截图则用 `Alt+PrintScreen`。截图只是其中一种用法，任意按键和组合键都能互相映射。

> 注：少数系统版本会忽略模拟的 Win 组合键。若 `Win+Shift+S` 无响应，改用上面两个不含 Win 的写法即可。

整个程序是一个 **约 0.2 MB** 的单文件（实测 208,384 字节），常驻内存 **约 2 MB**（实测：任务管理器私有工作集 1.7 MB；含共享系统 DLL 的完整工作集 12 MB），无后台服务、无额外运行时依赖。

## 特性

- 全局按键重映射，基于 `WH_KEYBOARD_LL` 低层键盘钩子
- 支持组合键：`Ctrl+J -> F5`、`Ctrl+Shift+K -> Alt+F4`
- 用扫描码（scanCode）标识物理键位，区分左右 Ctrl/Alt，不受键盘布局与输入法影响
- 配置热更新：改动规则立即生效，无需重启或注销
- 托盘常驻 + 图形界面添加/删除规则，按键捕获录制
- 开机自启（`HKCU\...\Run`）
- Rust + `windows-sys`，无 UI 框架、无 serde，唯一外部依赖

## 构建

需要 Rust 的 Windows MSVC 工具链：

```bash
cargo build --release
```

产物：`target/release/keyremapper.exe`（约 0.2 MB，单文件）。

```bash
cargo test            # 单元测试
cargo run -- --smoke  # 验证钩子与托盘初始化后立即退出
```

## 使用

1. 运行 `keyremapper.exe`，系统托盘出现图标。
2. 右键 →「编辑映射…」→ 点「添加映射」。
3. 先按触发组合键（如 `Ctrl+J`），再按目标组合键（如 `F5`）；按 `Esc` 取消。
4. 规则自动保存并立即生效。

托盘菜单还可暂停/启用映射、开关开机自启、退出程序。

## 配置

路径：`%APPDATA%\KeyRemapper\config.txt`

```text
enabled=true
Ctrl+J=F5
Ctrl+Shift+K=Alt+F4
```

修饰键 `Ctrl` / `Shift` / `Alt` / `Win` 用 `+` 连接，末项必须是主键（非修饰键）。也可写具体键名 `LCtrl` / `RCtrl` / `LAlt` / `RAlt` / `LWin` / `RWin`。未知按键以 `SC` + 十六进制扫描码表示（如 `SC154`），保证任意物理键都能无损存取。配置文件中的无效行会在启动时提示，不会静默丢弃。

## 实现要点

- 钩子装在专用线程并带消息循环。`LowLevelKeyboardProc` 超时会被 Windows 静默卸载，因此回调内只做内存查表 + 一次批量 `SendInput`：不做文件 IO、不等待锁、不弹窗。
- 命中组合键时，先抬起用户真实按下的修饰键，注入目标序列后再按原顺序恢复。否则 `Ctrl+J -> F5` 会被目标程序看成 `Ctrl+F5`。
- 注入事件用 `dwExtraInfo` 打标记，防止钩子回环。
- 规则表用 `RwLock`，回调以 `try_read` 读取；写锁被占用时直接放行，绝不阻塞输入。

## 已知限制

- **UIPI**：普通权限进程拦不住管理员权限窗口的按键。需要覆盖时用「任务计划程序」以最高权限启动本程序。
- 无法拦截 `Ctrl+Alt+Del`、UAC 安全桌面，以及部分使用 Raw Input / 内核反作弊的游戏。
- Win 键按下时系统会先弹出开始菜单；AltGr 会产生 `LCtrl + RAlt` 双事件，可能误触发 `Ctrl+Alt` 类规则。
- 需要驱动级方案（如 Interception）才能全覆盖，但会引入驱动签名与安装权限要求，与「轻量、免安装」目标冲突，本项目不做。

## 免责说明

本工具会全局拦截键盘输入。请自行确认映射规则不会覆盖系统快捷键；作者不对误操作或数据损失负责。

## 许可证

MIT — 见 [LICENSE](LICENSE)。
