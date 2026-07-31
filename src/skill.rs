use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use directories::BaseDirs;

pub const MIXLY_SKILL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/skills/mixly/SKILL.md"
));

#[derive(Debug, Clone, Copy, ValueEnum)]
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
}
