//! The agent skill, embedded in the binary and installable with
//! `depgraph skill install`.
//!
//! The skill follows the open Agent Skills layout (a folder with `SKILL.md`),
//! which Claude Code, OpenCode, Pi and others read. Each agent looks in its
//! own folder, so installing means copying the same files to the right place.
//! The files are compiled in: the installed skill always matches the binary.

use std::path::{Path, PathBuf};

use crate::error::Error;

/// Skill folder name. Must equal `name:` in `SKILL.md`.
pub const NAME: &str = "analyzing-service-dependencies";

/// File written next to the skill so an update knows the folder is ours.
const MARKER: &str = ".installed-by-depgraph";

/// Every file of the skill: path inside the skill folder, contents.
const FILES: [(&str, &str); 4] = [
    (
        "SKILL.md",
        include_str!("../skills/analyzing-service-dependencies/SKILL.md"),
    ),
    (
        "references/commands.md",
        include_str!("../skills/analyzing-service-dependencies/references/commands.md"),
    ),
    (
        "references/reading-results.md",
        include_str!("../skills/analyzing-service-dependencies/references/reading-results.md"),
    ),
    (
        "references/limits.md",
        include_str!("../skills/analyzing-service-dependencies/references/limits.md"),
    ),
];

/// A coding agent that reads skills from a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Agent {
    /// Claude Code.
    Claude,
    /// OpenCode.
    Opencode,
    /// Pi coding agent.
    Pi,
    /// The shared `.agents/skills` folder. Read by OpenCode, Pi and other
    /// agents that follow the Agent Skills layout.
    Agents,
}

impl Agent {
    /// All targets, in the order they are listed.
    pub const ALL: [Self; 4] = [Self::Claude, Self::Opencode, Self::Pi, Self::Agents];

    /// Name as typed on the command line.
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Opencode => "opencode",
            Self::Pi => "pi",
            Self::Agents => "agents",
        }
    }

    /// Folder that holds skills, for the user (`global`) or one project.
    pub fn skills_dir(self, home: &Path, project: Option<&Path>) -> PathBuf {
        match (self, project) {
            (Self::Claude, None) => home.join(".claude/skills"),
            (Self::Claude, Some(root)) => root.join(".claude/skills"),
            (Self::Opencode, None) => config_home(home).join("opencode/skills"),
            (Self::Opencode, Some(root)) => root.join(".opencode/skills"),
            (Self::Pi, None) => home.join(".pi/agent/skills"),
            (Self::Pi, Some(root)) => root.join(".pi/skills"),
            (Self::Agents, None) => home.join(".agents/skills"),
            (Self::Agents, Some(root)) => root.join(".agents/skills"),
        }
    }

    /// Folder whose presence says the agent is installed for this user.
    fn marker_dir(self, home: &Path) -> PathBuf {
        match self {
            Self::Claude => home.join(".claude"),
            Self::Opencode => config_home(home).join("opencode"),
            Self::Pi => home.join(".pi"),
            Self::Agents => home.join(".agents"),
        }
    }
}

fn config_home(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
}

/// The user's home folder.
pub fn home_dir() -> Result<PathBuf, Error> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            Error::Usage("cannot find your home folder; pass --dir <skills folder>\n".to_owned())
        })
}

/// Agents to install for when the user named none: the ones found on this
/// machine, or the shared folder when none is found.
pub fn detect(home: &Path) -> Vec<Agent> {
    let found: Vec<Agent> = [Agent::Claude, Agent::Opencode, Agent::Pi]
        .into_iter()
        .filter(|agent| agent.marker_dir(home).is_dir())
        .collect();
    if found.is_empty() {
        vec![Agent::Agents]
    } else {
        found
    }
}

/// What happened to one target.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Newly written.
    Installed,
    /// Replaced an earlier install of ours.
    Updated,
    /// Same files already there.
    Current,
    /// A folder of that name exists that depgraph did not write.
    Refused,
}

/// Result for one folder.
#[derive(Debug)]
pub struct Installed {
    /// The skill folder (`<skills dir>/analyzing-service-dependencies`).
    pub path: PathBuf,
    /// What happened.
    pub outcome: Outcome,
}

fn stamp() -> String {
    format!("depgraph {}\n", env!("CARGO_PKG_VERSION"))
}

fn is_current(target: &Path) -> bool {
    FILES.iter().all(|(rel, body)| {
        std::fs::read_to_string(target.join(rel)).is_ok_and(|have| have == *body)
    }) && std::fs::read_to_string(target.join(MARKER)).is_ok_and(|have| have == stamp())
}

/// Copy the skill into `<skills_dir>/<NAME>`.
///
/// A folder depgraph did not write is left alone unless `force` is set.
/// With `dry_run` nothing is written, and the outcome says what would happen.
pub fn install(skills_dir: &Path, force: bool, dry_run: bool) -> Result<Installed, Error> {
    let target = skills_dir.join(NAME);
    let exists = target.exists();
    let ours = target.join(MARKER).is_file();
    let outcome = if !exists {
        Outcome::Installed
    } else if is_current(&target) {
        Outcome::Current
    } else if ours || force {
        Outcome::Updated
    } else {
        Outcome::Refused
    };
    if dry_run || matches!(outcome, Outcome::Current | Outcome::Refused) {
        return Ok(Installed {
            path: target,
            outcome,
        });
    }
    for (rel, body) in FILES {
        let file = target.join(rel);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file, body)?;
    }
    std::fs::write(target.join(MARKER), stamp())?;
    Ok(Installed {
        path: target,
        outcome,
    })
}

/// Remove an install of ours. Returns false when there was none.
pub fn uninstall(skills_dir: &Path, force: bool) -> Result<bool, Error> {
    let target = skills_dir.join(NAME);
    if !target.exists() {
        return Ok(false);
    }
    if !target.join(MARKER).is_file() && !force {
        return Err(Error::Usage(format!(
            "{} was not installed by depgraph; pass --force to remove it\n",
            target.display()
        )));
    }
    std::fs::remove_dir_all(&target)?;
    Ok(true)
}

/// The main file of the skill, for `depgraph skill print`.
pub fn skill_md() -> &'static str {
    FILES[0].1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frontmatter(text: &str) -> (String, String) {
        let body = text.strip_prefix("---\n").expect("frontmatter");
        let end = body.find("\n---").expect("frontmatter end");
        let mut name = String::new();
        let mut description = String::new();
        for line in body[..end].lines() {
            if let Some(value) = line.strip_prefix("name:") {
                name = value.trim().to_owned();
            }
            if let Some(value) = line.strip_prefix("description:") {
                description = value.trim().to_owned();
            }
        }
        (name, description)
    }

    #[test]
    fn the_embedded_skill_follows_the_authoring_limits() {
        let (name, description) = frontmatter(skill_md());
        assert_eq!(name, NAME);
        assert!(name.len() <= 64);
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert!(!name.contains("claude") && !name.contains("anthropic"));
        assert!(!description.is_empty() && description.len() <= 1024);
        assert!(skill_md().lines().count() < 500);
    }

    #[test]
    fn every_file_the_skill_links_is_embedded() {
        for (rel, _) in &FILES[1..] {
            assert!(skill_md().contains(rel), "SKILL.md does not link {rel}");
        }
    }

    #[test]
    fn install_writes_updates_and_refuses_foreign_folders() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = install(dir.path(), false, false).expect("install");
        assert_eq!(first.outcome, Outcome::Installed);
        assert!(first.path.join("references/limits.md").is_file());

        let again = install(dir.path(), false, false).expect("again");
        assert_eq!(again.outcome, Outcome::Current);

        std::fs::write(first.path.join("SKILL.md"), "edited").expect("edit");
        let update = install(dir.path(), false, false).expect("update");
        assert_eq!(update.outcome, Outcome::Updated);
        assert_eq!(
            std::fs::read_to_string(update.path.join("SKILL.md")).expect("read"),
            skill_md()
        );

        let other = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(other.path().join(NAME)).expect("mkdir");
        std::fs::write(other.path().join(NAME).join("SKILL.md"), "mine").expect("write");
        let refused = install(other.path(), false, false).expect("refused");
        assert_eq!(refused.outcome, Outcome::Refused);
        assert_eq!(
            std::fs::read_to_string(other.path().join(NAME).join("SKILL.md")).expect("read"),
            "mine"
        );
        assert_eq!(
            install(other.path(), true, false).expect("force").outcome,
            Outcome::Updated
        );
    }

    #[test]
    fn dry_run_writes_nothing_and_uninstall_only_removes_ours() {
        let dir = tempfile::tempdir().expect("tempdir");
        let planned = install(dir.path(), false, true).expect("dry run");
        assert_eq!(planned.outcome, Outcome::Installed);
        assert!(!planned.path.exists());

        install(dir.path(), false, false).expect("install");
        assert!(uninstall(dir.path(), false).expect("remove"));
        assert!(!uninstall(dir.path(), false).expect("none left"));

        std::fs::create_dir_all(dir.path().join(NAME)).expect("mkdir");
        assert!(uninstall(dir.path(), false).is_err());
    }

    #[test]
    fn each_agent_reads_its_documented_folder() {
        let home = Path::new("/home/u");
        let project = Path::new("/work/app");
        assert_eq!(
            Agent::Claude.skills_dir(home, None),
            home.join(".claude/skills")
        );
        assert_eq!(
            Agent::Pi.skills_dir(home, None),
            home.join(".pi/agent/skills")
        );
        assert_eq!(
            Agent::Agents.skills_dir(home, None),
            home.join(".agents/skills")
        );
        assert_eq!(
            Agent::Claude.skills_dir(home, Some(project)),
            project.join(".claude/skills")
        );
        assert_eq!(
            Agent::Opencode.skills_dir(home, Some(project)),
            project.join(".opencode/skills")
        );
        assert_eq!(
            Agent::Pi.skills_dir(home, Some(project)),
            project.join(".pi/skills")
        );
    }

    #[test]
    fn detection_falls_back_to_the_shared_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(detect(dir.path()), vec![Agent::Agents]);
        std::fs::create_dir_all(dir.path().join(".pi")).expect("mkdir");
        assert_eq!(detect(dir.path()), vec![Agent::Pi]);
    }
}
