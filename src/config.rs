//! 规则模型与配置读写。
//!
//! 配置采用纯文本行格式，避免为解析 JSON 引入额外依赖：
//!   enabled=true
//!   Ctrl+J=F5
//!   Ctrl+Shift+K=Alt+F4

use crate::hook;
use crate::keys::{self, KeyId};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub type RuleMap = HashMap<(u8, KeyId), Vec<KeyId>>;

#[derive(Clone, Debug)]
pub struct Rule {
    pub mods: u8,
    pub trigger: KeyId,
    pub output: Vec<KeyId>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub enabled: bool,
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true, rules: Vec::new() }
    }
}

static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();

pub fn init(cfg: Config) {
    let _ = CONFIG.set(Mutex::new(cfg));
}

pub fn with<R>(f: impl FnOnce(&mut Config) -> R) -> R {
    let mut guard = CONFIG.get().expect("config 未初始化").lock().unwrap();
    f(&mut guard)
}

pub fn snapshot() -> Config {
    CONFIG.get().expect("config 未初始化").lock().unwrap().clone()
}

/// 保存配置并让钩子线程立即使用新规则表。
pub fn commit() -> io::Result<()> {
    let cfg = snapshot();
    cfg.save()?;
    hook::apply(&cfg);
    Ok(())
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    base.join("KeyRemapper").join("config.txt")
}

impl Config {
    pub fn to_map(&self) -> RuleMap {
        self.rules
            .iter()
            .map(|r| ((r.mods, r.trigger), r.output.clone()))
            .collect()
    }

    pub fn rule_text(&self, r: &Rule) -> String {
        let from = keys::format_combo(&keys::combo_from_signature(r.mods, r.trigger));
        let to = keys::format_combo(&r.output);
        format!("{from}  ->  {to}")
    }

    /// 读取配置。非法行跳过并通过 warnings 返回，不静默丢弃。
    pub fn load() -> (Config, Vec<String>) {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(s) => Config::from_text(&s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // 首次运行写入带说明的模板，方便用户手工编辑。
                let cfg = Config::default();
                let _ = cfg.save();
                (cfg, Vec::new())
            }
            Err(e) => (Config::default(), vec![format!("读取配置失败: {e}")]),
        }
    }

    pub fn from_text(content: &str) -> (Config, Vec<String>) {
        let mut cfg = Config::default();
        let mut warnings = Vec::new();
        for (idx, raw) in content.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(v) = line.strip_prefix("enabled=") {
                cfg.enabled = v.trim().eq_ignore_ascii_case("true");
                continue;
            }
            let Some((lhs, rhs)) = line.split_once('=') else {
                warnings.push(format!("第 {} 行缺少 '='，已跳过", idx + 1));
                continue;
            };
            match (keys::parse_combo(lhs), keys::parse_combo(rhs)) {
                (Ok((mods, trigger, _)), Ok((_, _, output))) => {
                    cfg.rules.retain(|r| !(r.mods == mods && r.trigger == trigger));
                    cfg.rules.push(Rule { mods, trigger, output });
                }
                (Err(e), _) | (_, Err(e)) => {
                    warnings.push(format!("第 {} 行无效（{}），已跳过", idx + 1, e));
                }
            }
        }
        (cfg, warnings)
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from(
            "# KeyRemapper 配置\n\
             # 每行一条规则: 触发组合=输出组合\n\
             # 修饰键 Ctrl/Shift/Alt/Win 用 + 连接，末项为主键\n\
             # 例: Ctrl+J=F5    CapsLock=Ctrl+Space\n",
        );
        out.push_str(&format!("enabled={}\n", self.enabled));
        for r in &self.rules {
            let from = keys::format_combo(&keys::combo_from_signature(r.mods, r.trigger));
            let to = keys::format_combo(&r.output);
            out.push_str(&from);
            out.push('=');
            out.push_str(&to);
            out.push('\n');
        }
        out
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, self.to_text())
    }
}

/// 新增/覆盖一条规则。同签名只保留最后一条。
pub fn add_rule(trigger: &[KeyId], output: &[KeyId]) -> Result<(), String> {
    let key = *trigger.last().ok_or("触发组合为空")?;
    if keys::is_modifier(key) {
        return Err("触发组合的末项不能是修饰键".into());
    }
    let mods = keys::mods_of(trigger);
    with(|cfg| {
        cfg.rules.retain(|r| !(r.mods == mods && r.trigger == key));
        cfg.rules.push(Rule { mods, trigger: key, output: output.to_vec() });
    });
    commit().map_err(|e| format!("保存失败: {e}"))
}

pub fn remove_rule(index: usize) -> Result<(), String> {
    with(|cfg| {
        if index < cfg.rules.len() {
            cfg.rules.remove(index);
        }
    });
    commit().map_err(|e| format!("保存失败: {e}"))
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    with(|cfg| cfg.enabled = enabled);
    commit().map_err(|e| format!("保存失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{MOD_ALT, MOD_CTRL};

    #[test]
    fn text_round_trip() {
        let cfg = Config {
            enabled: false,
            rules: vec![
                Rule { mods: MOD_CTRL, trigger: 0x024, output: vec![0x03F] },
                Rule { mods: MOD_ALT, trigger: 0x025, output: vec![0x038, 0x03E] },
            ],
        };
        let (back, warnings) = Config::from_text(&cfg.to_text());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!back.enabled);
        assert_eq!(back.rules.len(), 2);
        assert_eq!(back.rules[0].mods, MOD_CTRL);
        assert_eq!(back.rules[0].trigger, 0x024);
        assert_eq!(back.rules[0].output, vec![0x03F]);
        assert_eq!(back.rules[1].output, vec![0x038, 0x03E]);
        assert_eq!(back.to_map().len(), 2);
    }

    #[test]
    fn invalid_lines_are_reported_not_silently_dropped() {
        let (cfg, warnings) = Config::from_text("enabled=true\nCtrl+J=F5\nBROKEN\nJ+K=F5\n");
        assert_eq!(cfg.rules.len(), 1);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn duplicate_signature_keeps_last_rule() {
        let (cfg, _) = Config::from_text("Ctrl+J=F5\nCtrl+J=F6\n");
        assert_eq!(cfg.rules.len(), 1);
        assert_eq!(cfg.rules[0].output, vec![0x040]);
    }
}
