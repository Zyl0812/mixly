use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use directories::BaseDirs;
use serde_json::Value;

pub const MIXLY_SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/skills/mixly/SKILL.md"
));

/// Mixly 写入 Claude Code `~/.claude/settings.json` 的 statusLine 配置。
/// 卸载时只有完全匹配本值才会被移除，避免删除用户后来修改的配置。
pub const MIXLY_STATUS_LINE: &str =
    r#"{"type":"command","command":"mixly status --claude","refreshInterval":1}"#;
const MIXLY_STATUS_COMMAND: &str = "mixly status --claude";
const MIXLY_STATUS_PREFIX: &str = "mixly status --claude; ";

const CLAUDE_MARKETPLACE_NAME: &str = "mixly-local";
const CLAUDE_PLUGIN_ID: &str = "mixly@mixly-local";
const CLAUDE_MARKETPLACE: &str = r#"{
  "name": "mixly-local",
  "owner": { "name": "Mixly" },
  "plugins": [
    {
      "name": "mixly",
      "source": "./plugins/mixly",
      "description": "Mixly playback skill and Claude Code status line"
    }
  ]
}"#;
const CLAUDE_PLUGIN_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/claude-plugin/.claude-plugin/plugin.json"
));
const CLAUDE_PLAY_SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/claude-plugin/skills/play/SKILL.md"
));
const CLAUDE_MIXLY_SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/claude-plugin/skills/mixly/SKILL.md"
));

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SkillAgent {
    Claude,
    Codex,
    Grok,
}

#[derive(Debug, Clone, Copy)]
pub enum SkillScope {
    Global,
    Project,
}

pub fn skill_file_path(agent: SkillAgent, scope: SkillScope) -> Result<PathBuf> {
    let base = match scope {
        SkillScope::Global => BaseDirs::new()
            .context("无法确定用户主目录")?
            .home_dir()
            .to_path_buf(),
        SkillScope::Project => std::env::current_dir().context("无法确定当前目录")?,
    };
    Ok(skill_file_path_from_base(&base, agent))
}

pub fn install_skill(agent: SkillAgent, scope: SkillScope, force: bool) -> Result<(PathBuf, bool)> {
    let path = skill_file_path(agent, scope)?;
    let changed = install_at(&path, force)?;
    Ok((path, changed))
}

pub fn uninstall_skill(
    agent: SkillAgent,
    scope: SkillScope,
    force: bool,
) -> Result<(PathBuf, bool)> {
    let path = skill_file_path(agent, scope)?;
    let changed = uninstall_at(&path, force)?;
    Ok((path, changed))
}

fn skill_file_path_from_base(base: &Path, agent: SkillAgent) -> PathBuf {
    let root = match agent {
        SkillAgent::Claude => ".claude",
        SkillAgent::Codex => ".agents",
        SkillAgent::Grok => ".grok",
    };
    base.join(root)
        .join("skills")
        .join("mixly")
        .join("SKILL.md")
}

fn install_at(path: &Path, force: bool) -> Result<bool> {
    if path.exists() {
        let current = fs::read(path).with_context(|| format!("读取 {}", path.display()))?;
        if current == MIXLY_SKILL.as_bytes() {
            return Ok(false);
        }
        if !force {
            bail!("{} 已存在且内容不同；确认覆盖请加 --force", path.display());
        }
    }

    let parent = path.parent().context("Skill 路径没有父目录")?;
    fs::create_dir_all(parent).with_context(|| format!("创建 {}", parent.display()))?;
    fs::write(path, MIXLY_SKILL).with_context(|| format!("写入 {}", path.display()))?;
    Ok(true)
}

fn uninstall_at(path: &Path, force: bool) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }

    let current = fs::read(path).with_context(|| format!("读取 {}", path.display()))?;
    if current != MIXLY_SKILL.as_bytes() && !force {
        bail!("{} 已被修改；确认删除请加 --force", path.display());
    }

    fs::remove_file(path).with_context(|| format!("删除 {}", path.display()))?;
    if let Some(parent) = path.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(true)
}

// ── Claude Code 插件 + status line（Plan：Claude Code 播放状态插件） ──────

fn claude_home() -> Result<PathBuf> {
    Ok(BaseDirs::new()
        .context("无法确定用户主目录")?
        .home_dir()
        .join(".claude"))
}

fn claude_settings_path() -> Result<PathBuf> {
    Ok(claude_home()?.join("settings.json"))
}

fn claude_status_line_backup_path() -> Result<PathBuf> {
    Ok(claude_home()?.join("mixly-statusline-backup.json"))
}

/// 本地 marketplace 源目录；Claude Code 会把安装版本复制到自己的插件缓存。
fn claude_marketplace_dir() -> Result<PathBuf> {
    Ok(claude_home()?.join("mixly-marketplace"))
}

fn legacy_claude_plugin_dir() -> Result<PathBuf> {
    Ok(claude_home()?.join("plugins").join("mixly"))
}

fn marketplace_files(root: &Path) -> [(PathBuf, &'static str); 4] {
    [
        (
            root.join(".claude-plugin").join("marketplace.json"),
            CLAUDE_MARKETPLACE,
        ),
        (
            root.join("plugins")
                .join("mixly")
                .join(".claude-plugin")
                .join("plugin.json"),
            CLAUDE_PLUGIN_MANIFEST,
        ),
        (
            root.join("plugins")
                .join("mixly")
                .join("skills")
                .join("mixly")
                .join("SKILL.md"),
            CLAUDE_MIXLY_SKILL,
        ),
        (
            root.join("plugins")
                .join("mixly")
                .join("skills")
                .join("play")
                .join("SKILL.md"),
            CLAUDE_PLAY_SKILL,
        ),
    ]
}

fn legacy_plugin_files(root: &Path) -> [(PathBuf, &'static str); 2] {
    [
        (
            root.join(".claude-plugin").join("plugin.json"),
            CLAUDE_PLUGIN_MANIFEST,
        ),
        (
            root.join("skills").join("play").join("SKILL.md"),
            CLAUDE_PLAY_SKILL,
        ),
    ]
}

fn preflight_marketplace_at(root: &Path, force: bool) -> Result<()> {
    for (path, expected) in marketplace_files(root) {
        if path.exists() && fs::read(&path)? != expected.as_bytes() && !force {
            bail!("{} 已存在且内容不同；确认覆盖请加 --force", path.display());
        }
    }
    Ok(())
}

fn stage_marketplace_at(root: &Path, force: bool) -> Result<bool> {
    preflight_marketplace_at(root, force)?;
    let mut changed = false;
    for (path, expected) in marketplace_files(root) {
        if path.exists() && fs::read(&path)? == expected.as_bytes() {
            continue;
        }
        let parent = path.parent().context("Claude 插件文件没有父目录")?;
        fs::create_dir_all(parent).with_context(|| format!("创建 {}", parent.display()))?;
        fs::write(&path, expected).with_context(|| format!("写入 {}", path.display()))?;
        changed = true;
    }
    Ok(changed)
}

fn remove_marketplace_at(root: &Path, force: bool) -> Result<bool> {
    preflight_marketplace_at(root, force)?;
    let mut changed = false;
    for (path, _) in marketplace_files(root) {
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("删除 {}", path.display()))?;
            changed = true;
        }
    }
    for dir in [
        root.join("plugins/mixly/skills/mixly"),
        root.join("plugins/mixly/skills/play"),
        root.join("plugins/mixly/skills"),
        root.join("plugins/mixly/.claude-plugin"),
        root.join("plugins/mixly"),
        root.join("plugins"),
        root.join(".claude-plugin"),
        root.to_path_buf(),
    ] {
        let _ = fs::remove_dir(dir);
    }
    Ok(changed)
}

fn preflight_legacy_plugin_at(root: &Path, force: bool) -> Result<()> {
    for (path, expected) in legacy_plugin_files(root) {
        if path.exists() && fs::read(&path)? != expected.as_bytes() && !force {
            bail!(
                "{} 已被修改；确认删除旧版插件文件请加 --force",
                path.display()
            );
        }
    }
    Ok(())
}

fn remove_legacy_plugin_at(root: &Path, force: bool) -> Result<bool> {
    preflight_legacy_plugin_at(root, force)?;
    let mut changed = false;
    for (path, _) in legacy_plugin_files(root) {
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("删除 {}", path.display()))?;
            changed = true;
        }
    }
    for dir in [
        root.join("skills/play"),
        root.join("skills"),
        root.join(".claude-plugin"),
        root.to_path_buf(),
    ] {
        let _ = fs::remove_dir(dir);
    }
    Ok(changed)
}

fn run_claude(args: &[&str]) -> Result<String> {
    let output = Command::new("claude")
        .args(args)
        .output()
        .context("未找到 Claude Code CLI；请先安装并确保 claude 在 PATH 中")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        bail!("claude {} 失败: {detail}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn claude_list_contains(args: &[&str], field: &str, value: &str) -> Result<bool> {
    let value_json: Value =
        serde_json::from_str(&run_claude(args)?).context("解析 Claude Code 插件列表失败")?;
    Ok(value_json
        .as_array()
        .into_iter()
        .flatten()
        .any(|item| item.get(field).and_then(Value::as_str) == Some(value)))
}

fn claude_plugin_installed() -> Result<bool> {
    claude_list_contains(&["plugin", "list", "--json"], "id", CLAUDE_PLUGIN_ID)
}

fn claude_marketplace_registered() -> Result<bool> {
    claude_list_contains(
        &["plugin", "marketplace", "list", "--json"],
        "name",
        CLAUDE_MARKETPLACE_NAME,
    )
}

fn read_json(path: &Path, context: &str) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("读取 {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| context.to_string())
}

fn status_command(status: &Value) -> Result<&str> {
    let object = status.as_object().context("Claude statusLine 必须是对象")?;
    if object.get("type").and_then(Value::as_str) != Some("command") {
        bail!("Claude statusLine 不是 command 类型，无法安全组合");
    }
    object
        .get("command")
        .and_then(Value::as_str)
        .context("Claude statusLine 缺少 command")
}

fn compose_status_line(status: &Value) -> Result<Value> {
    let command = status_command(status)?;
    if command == MIXLY_STATUS_COMMAND || command.starts_with(MIXLY_STATUS_PREFIX) {
        return Ok(status.clone());
    }
    let mut combined = status.clone();
    let object = combined.as_object_mut().expect("validated above");
    object.insert(
        "command".to_string(),
        Value::String(format!("{MIXLY_STATUS_PREFIX}{command}")),
    );
    // 外部播放状态需要在 Claude 空闲时继续刷新；原值完整保存在备份中。
    object.insert("refreshInterval".to_string(), Value::from(1));
    Ok(combined)
}

fn preflight_status_line_install_at(path: &Path, backup_path: &Path, force: bool) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let cfg = read_json(path, "解析 settings.json 失败（请人工修复后重试）")?;
    let ours: Value = serde_json::from_str(MIXLY_STATUS_LINE)?;
    let Some(status) = cfg.get("statusLine") else {
        return Ok(());
    };
    if *status == ours {
        return Ok(());
    }
    let command = status_command(status)?;
    if command.starts_with(MIXLY_STATUS_PREFIX) {
        let backup = read_json(backup_path, "解析 Mixly statusLine 备份失败")?;
        if compose_status_line(&backup)? != *status && !force {
            bail!("组合 statusLine 已被修改；确认重新组合请加 --force");
        }
    } else if backup_path.exists() {
        let backup = read_json(backup_path, "解析 Mixly statusLine 备份失败")?;
        if backup != *status && !force {
            bail!(
                "{} 已有不同的 statusLine 备份；确认覆盖请加 --force",
                backup_path.display()
            );
        }
    }
    Ok(())
}

fn preflight_status_line_uninstall_at(path: &Path, backup_path: &Path, force: bool) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let cfg = read_json(path, "解析 settings.json 失败")?;
    let Some(status) = cfg.get("statusLine") else {
        return Ok(());
    };
    let Ok(command) = status_command(status) else {
        return Ok(()); // 非 Mixly 配置不属于卸载范围
    };
    if command.starts_with(MIXLY_STATUS_PREFIX) {
        if !backup_path.exists() && !force {
            bail!("Mixly statusLine 备份不存在，未自动修改现有配置");
        }
        if backup_path.exists() {
            let backup = read_json(backup_path, "解析 Mixly statusLine 备份失败")?;
            if compose_status_line(&backup)? != *status && !force {
                bail!("组合 statusLine 已被修改；确认移除 Mixly 前缀请加 --force");
            }
        }
    }
    Ok(())
}

/// 在 `settings.json` 中把 Mixly 输出放在已有 statusLine 之前。
/// 原 statusLine 完整备份，卸载时精确恢复。
fn configure_status_line_at(path: &Path, backup_path: &Path, force: bool) -> Result<bool> {
    let mut cfg: Value = if path.exists() {
        read_json(path, "解析 settings.json 失败（请人工修复后重试）")?
    } else {
        Value::Object(Default::default())
    };
    let ours: Value = serde_json::from_str(MIXLY_STATUS_LINE)?;

    let Some(existing) = cfg.get("statusLine").cloned() else {
        cfg["statusLine"] = ours;
        write_settings(path, &cfg)?;
        return Ok(true);
    };
    if existing == ours {
        return Ok(false);
    }
    if status_command(&existing)?.starts_with(MIXLY_STATUS_PREFIX) {
        preflight_status_line_install_at(path, backup_path, force)?;
        return Ok(false);
    }
    if backup_path.exists() && !force {
        bail!(
            "{} 已存在；确认覆盖旧备份请加 --force",
            backup_path.display()
        );
    }
    write_settings(backup_path, &existing)?;
    cfg["statusLine"] = compose_status_line(&existing)?;
    write_settings(path, &cfg)?;
    Ok(true)
}

/// 移除 Mixly 独占 statusLine，或从组合命令中移除 Mixly 并恢复原配置。
fn remove_status_line_at(path: &Path, backup_path: &Path, force: bool) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut cfg = read_json(path, "解析 settings.json 失败")?;
    let Some(status) = cfg.get("statusLine").cloned() else {
        return Ok(false);
    };
    let ours: Value = serde_json::from_str(MIXLY_STATUS_LINE)?;
    if status == ours {
        cfg.as_object_mut()
            .context("settings.json 不是对象")?
            .remove("statusLine");
        write_settings(path, &cfg)?;
        return Ok(true);
    }
    let Ok(command) = status_command(&status) else {
        return Ok(false);
    };
    if !command.starts_with(MIXLY_STATUS_PREFIX) {
        return Ok(false);
    }
    let original_command = command[MIXLY_STATUS_PREFIX.len()..].to_string();

    let restored = if backup_path.exists() {
        let backup = read_json(backup_path, "解析 Mixly statusLine 备份失败")?;
        if compose_status_line(&backup)? == status {
            backup
        } else if force {
            let mut current = status;
            let object = current.as_object_mut().expect("validated above");
            object.insert(
                "command".to_string(),
                Value::String(original_command.clone()),
            );
            match backup.get("refreshInterval") {
                Some(value) => object.insert("refreshInterval".to_string(), value.clone()),
                None => object.remove("refreshInterval"),
            };
            current
        } else {
            bail!("组合 statusLine 已被修改；确认移除 Mixly 前缀请加 --force");
        }
    } else if force {
        let mut current = status;
        current
            .as_object_mut()
            .expect("validated above")
            .insert("command".to_string(), Value::String(original_command));
        current
    } else {
        bail!("Mixly statusLine 备份不存在，未自动修改现有配置");
    };
    cfg["statusLine"] = restored;
    write_settings(path, &cfg)?;
    if backup_path.exists() {
        fs::remove_file(backup_path).with_context(|| format!("删除 {}", backup_path.display()))?;
    }
    Ok(true)
}

fn write_settings(path: &Path, cfg: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(cfg)?;
    fs::write(path, data).with_context(|| format!("写入 {}", path.display()))?;
    Ok(())
}

fn preflight_claude_plugin_files(force: bool) -> Result<()> {
    preflight_marketplace_at(&claude_marketplace_dir()?, force)?;
    preflight_legacy_plugin_at(&legacy_claude_plugin_dir()?, force)?;
    run_claude(&["--version"])?;
    Ok(())
}

/// 在修改 standalone skill 前检查 Claude 插件安装冲突与 CLI 可用性。
pub fn preflight_claude_plugin_install(force: bool) -> Result<()> {
    preflight_status_line_install_at(
        &claude_settings_path()?,
        &claude_status_line_backup_path()?,
        force,
    )?;
    preflight_claude_plugin_files(force)
}

/// 在修改 standalone skill 前检查 Claude 插件卸载是否可安全恢复。
pub fn preflight_claude_plugin_uninstall(force: bool) -> Result<()> {
    preflight_status_line_uninstall_at(
        &claude_settings_path()?,
        &claude_status_line_backup_path()?,
        force,
    )?;
    preflight_claude_plugin_files(force)
}

/// 只配置 Claude Code 主状态栏，不安装或替换插件。
pub fn install_claude_status_line(force: bool) -> Result<(PathBuf, bool)> {
    run_claude(&["--version"])?;
    let settings = claude_settings_path()?;
    let backup = claude_status_line_backup_path()?;
    let changed = configure_status_line_at(&settings, &backup, force)?;
    Ok((settings, changed))
}

/// 只移除 Mixly 状态栏，保留插件和其他 Claude Code 配置。
pub fn uninstall_claude_status_line(force: bool) -> Result<(PathBuf, bool)> {
    let settings = claude_settings_path()?;
    let backup = claude_status_line_backup_path()?;
    let changed = remove_status_line_at(&settings, &backup, force)?;
    Ok((settings, changed))
}

/// 安装 Claude 插件：生成本地 marketplace、交给 Claude Code 注册，再配置 statusLine。
pub fn install_claude_plugin(force: bool) -> Result<(PathBuf, bool)> {
    let settings = claude_settings_path()?;
    let status_backup = claude_status_line_backup_path()?;
    let marketplace = claude_marketplace_dir()?;
    let legacy = legacy_claude_plugin_dir()?;
    preflight_claude_plugin_install(force)?;

    let staged = stage_marketplace_at(&marketplace, force)?;
    let marketplace_arg = marketplace.to_string_lossy().into_owned();
    run_claude(&[
        "plugin",
        "marketplace",
        "add",
        &marketplace_arg,
        "--scope",
        "user",
    ])?;
    let installed = claude_plugin_installed()?;
    if staged && installed {
        run_claude(&["plugin", "uninstall", CLAUDE_PLUGIN_ID, "--scope", "user"])?;
    }
    if staged || !installed {
        run_claude(&["plugin", "install", CLAUDE_PLUGIN_ID, "--scope", "user"])?;
    }
    let status_changed = configure_status_line_at(&settings, &status_backup, force)?;
    let legacy_changed = remove_legacy_plugin_at(&legacy, force)?;
    Ok((
        marketplace,
        staged || !installed || status_changed || legacy_changed,
    ))
}

/// 卸载 Claude 插件、marketplace 与完全匹配的本地文件/statusLine。
pub fn uninstall_claude_plugin(force: bool) -> Result<(PathBuf, bool)> {
    let settings = claude_settings_path()?;
    let status_backup = claude_status_line_backup_path()?;
    let marketplace = claude_marketplace_dir()?;
    let legacy = legacy_claude_plugin_dir()?;
    preflight_claude_plugin_uninstall(force)?;

    let installed = claude_plugin_installed()?;
    if installed {
        run_claude(&["plugin", "uninstall", CLAUDE_PLUGIN_ID, "--scope", "user"])?;
    }
    let registered = claude_marketplace_registered()?;
    if registered {
        run_claude(&[
            "plugin",
            "marketplace",
            "remove",
            CLAUDE_MARKETPLACE_NAME,
            "--scope",
            "user",
        ])?;
    }
    let status_changed = remove_status_line_at(&settings, &status_backup, force)?;
    let files_changed = remove_marketplace_at(&marketplace, force)?;
    let legacy_changed = remove_legacy_plugin_at(&legacy, force)?;
    Ok((
        marketplace,
        installed || registered || status_changed || files_changed || legacy_changed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn agent_paths_match_official_layouts() {
        let base = Path::new("workspace");
        assert_eq!(
            skill_file_path_from_base(base, SkillAgent::Claude),
            base.join(".claude")
                .join("skills")
                .join("mixly")
                .join("SKILL.md")
        );
        assert_eq!(
            skill_file_path_from_base(base, SkillAgent::Codex),
            base.join(".agents")
                .join("skills")
                .join("mixly")
                .join("SKILL.md")
        );
        assert_eq!(
            skill_file_path_from_base(base, SkillAgent::Grok),
            base.join(".grok")
                .join("skills")
                .join("mixly")
                .join("SKILL.md")
        );
    }

    #[test]
    fn install_and_uninstall_protect_modified_skill() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mixly").join("SKILL.md");

        assert!(install_at(&path, false).unwrap());
        assert!(!install_at(&path, false).unwrap());

        fs::write(&path, "user edit").unwrap();
        assert!(install_at(&path, false).is_err());
        assert!(uninstall_at(&path, false).is_err());

        assert!(install_at(&path, true).unwrap());
        fs::write(&path, "user edit").unwrap();
        assert!(uninstall_at(&path, true).unwrap());
        assert!(!path.exists());
        assert!(!uninstall_at(&path, false).unwrap());
    }

    #[test]
    fn status_line_composes_idempotently_and_restores_original() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let backup = dir.path().join("status-backup.json");
        fs::write(
            &path,
            r#"{"theme":"dark","statusLine":{"type":"command","command":"if [ -f \"$HOME/orca.sh\" ]; then \"$HOME/orca.sh\"; fi","padding":2,"refreshInterval":5}}"#,
        )
        .unwrap();
        let original: Value = serde_json::json!({
            "type": "command",
            "command": "if [ -f \"$HOME/orca.sh\" ]; then \"$HOME/orca.sh\"; fi",
            "padding": 2,
            "refreshInterval": 5
        });

        assert!(configure_status_line_at(&path, &backup, false).unwrap());
        let cfg: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg["theme"], "dark");
        assert_eq!(
            cfg["statusLine"]["command"],
            "mixly status --claude; if [ -f \"$HOME/orca.sh\" ]; then \"$HOME/orca.sh\"; fi"
        );
        assert_eq!(cfg["statusLine"]["padding"], 2);
        assert_eq!(cfg["statusLine"]["refreshInterval"], 1);
        assert_eq!(read_json(&backup, "backup").unwrap(), original);
        assert!(!configure_status_line_at(&path, &backup, false).unwrap());

        assert!(remove_status_line_at(&path, &backup, false).unwrap());
        let cfg: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg["theme"], "dark");
        assert_eq!(cfg["statusLine"], original);
        assert!(!backup.exists());
    }

    #[test]
    fn force_uninstall_strips_mixly_but_preserves_later_user_edits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let backup = dir.path().join("status-backup.json");
        fs::write(
            &path,
            r#"{"statusLine":{"type":"command","command":"my-custom"}}"#,
        )
        .unwrap();
        configure_status_line_at(&path, &backup, false).unwrap();
        let mut cfg = read_json(&path, "settings").unwrap();
        cfg["statusLine"]["padding"] = Value::from(9);
        write_settings(&path, &cfg).unwrap();

        assert!(remove_status_line_at(&path, &backup, false).is_err());
        assert!(remove_status_line_at(&path, &backup, true).unwrap());
        let cfg = read_json(&path, "settings").unwrap();
        assert_eq!(cfg["statusLine"]["command"], "my-custom");
        assert_eq!(cfg["statusLine"]["padding"], 9);
        assert!(cfg["statusLine"].get("refreshInterval").is_none());
    }

    #[test]
    fn status_line_writes_into_fresh_settings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let backup = dir.path().join("status-backup.json");
        assert!(configure_status_line_at(&path, &backup, false).unwrap());
        let cfg: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            cfg["statusLine"],
            serde_json::from_str::<Value>(MIXLY_STATUS_LINE).unwrap()
        );
        assert!(!backup.exists());
        assert!(remove_status_line_at(&path, &backup, false).unwrap());
        let cfg = read_json(&path, "settings").unwrap();
        assert!(cfg.get("statusLine").is_none());
    }

    #[test]
    fn uninstall_leaves_unrelated_status_line_untouched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let backup = dir.path().join("status-backup.json");
        fs::write(
            &path,
            r#"{"statusLine":{"type":"command","command":"orca"}}"#,
        )
        .unwrap();
        assert!(!remove_status_line_at(&path, &backup, false).unwrap());
        assert_eq!(
            read_json(&path, "settings").unwrap()["statusLine"]["command"],
            "orca"
        );
    }

    #[test]
    fn marketplace_staging_is_idempotent_and_guards_changes() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("marketplace");

        assert!(stage_marketplace_at(&root, false).unwrap());
        assert!(!stage_marketplace_at(&root, false).unwrap());
        assert_eq!(
            fs::read_to_string(root.join("plugins/mixly/skills/mixly/SKILL.md")).unwrap(),
            MIXLY_SKILL
        );
        let skill = root.join("plugins/mixly/skills/play/SKILL.md");
        fs::write(&skill, "user edit").unwrap();
        assert!(stage_marketplace_at(&root, false).is_err());
        assert!(stage_marketplace_at(&root, true).unwrap());
        assert_eq!(fs::read_to_string(skill).unwrap(), CLAUDE_PLAY_SKILL);
    }

    #[test]
    fn public_plugin_mixly_skill_matches_standalone_skill() {
        assert_eq!(CLAUDE_MIXLY_SKILL, MIXLY_SKILL);
    }

    #[test]
    fn marketplace_uninstall_preserves_modified_and_extra_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("marketplace");
        stage_marketplace_at(&root, false).unwrap();
        let skill = root.join("plugins/mixly/skills/play/SKILL.md");
        let extra = root.join("user-note.txt");
        fs::write(&skill, "user edit").unwrap();
        fs::write(&extra, "keep").unwrap();

        assert!(remove_marketplace_at(&root, false).is_err());
        assert!(skill.exists());
        assert!(remove_marketplace_at(&root, true).unwrap());
        assert!(!skill.exists());
        assert!(extra.exists());
    }

    #[test]
    fn legacy_plugin_cleanup_requires_force_for_modified_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("legacy");
        for (path, expected) in legacy_plugin_files(&root) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, expected).unwrap();
        }
        let skill = root.join("skills/play/SKILL.md");
        fs::write(&skill, "user edit").unwrap();

        assert!(remove_legacy_plugin_at(&root, false).is_err());
        assert!(skill.exists());
        assert!(remove_legacy_plugin_at(&root, true).unwrap());
        assert!(!skill.exists());
    }

    #[test]
    fn embedded_plugin_version_matches_crate() {
        let manifest: Value = serde_json::from_str(CLAUDE_PLUGIN_MANIFEST).unwrap();
        assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    }
}
