//! Text of a file at git `HEAD`, used as the "before" side of the diff for
//! files a `bash` call changed — the loop's edit tools capture their own
//! pre-edit content, but bash edits bypass it (see `agent::tree_fingerprint`).

use std::path::Path;
use std::process::Command;

/// Content of `root`-relative `rel` at git `HEAD`, or `None` when there is
/// none: `root` is not inside a git repo, the repo has no commits, the file is
/// untracked at HEAD, or the HEAD content is not UTF-8. Never errors — a
/// lookup failure means "no before side", which is exactly what the diff wants.
///
/// `git show HEAD:<path>` resolves `<path>` from the repo top level, while the
/// project root may be a subdirectory of the repo; the `./` prefix resolves
/// from the current directory instead, so the command runs with `root` as its
/// working directory.
pub fn head_content(project_root: &Path, rel: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("show")
        .arg(format!("HEAD:./{}", rel.display()))
        .current_dir(project_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// A git repo with `committed` files committed at HEAD, in a `TempDir`.
    /// Every `git` call sets `GIT_CONFIG_GLOBAL=/dev/null` and
    /// `GIT_CONFIG_NOSYSTEM=1` so the host's config (identity, commit signing)
    /// cannot leak in, and passes the identity + `commit.gpgsign=false` on the
    /// commit itself.
    struct Repo {
        root: PathBuf,
    }

    impl Repo {
        fn init(dir: &tempfile::TempDir, committed: &[(&str, &str)]) -> Self {
            let root = dir.path().to_path_buf();
            for (name, body) in committed {
                let path = root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, body).unwrap();
            }
            let mut cmd = Command::new("git");
            cmd.args(["init", "--quiet"])
                .current_dir(&root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1");
            assert!(
                cmd.status().unwrap().success(),
                "git init failed in {}",
                root.display()
            );
            let mut add = Command::new("git");
            add.args(["add", "."])
                .current_dir(&root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1");
            assert!(add.status().unwrap().success(), "git add failed");
            let mut commit = Command::new("git");
            commit
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "-m",
                    "initial",
                ])
                .current_dir(&root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1");
            assert!(
                commit.status().unwrap().success(),
                "git commit failed in {}",
                root.display()
            );
            Self { root }
        }

        fn file(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }
    }

    #[test]
    fn head_content_returns_committed_text() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repo::init(&dir, &[("src/lib.rs", "original\n")]);

        fs::write(repo.file("src/lib.rs"), "rewritten on disk\n").unwrap();

        assert_eq!(
            head_content(&repo.root, Path::new("src/lib.rs")),
            Some("original\n".to_string()),
            "the lookup must return the committed text, not the working copy"
        );
    }

    #[test]
    fn head_content_is_none_for_untracked_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = Repo::init(&dir, &[("tracked.txt", "t\n")]);

        fs::write(repo.file("untracked.txt"), "new\n").unwrap();

        assert_eq!(head_content(&repo.root, Path::new("untracked.txt")), None);
    }

    #[test]
    fn head_content_is_none_outside_git_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("plain.txt"), "no git here\n").unwrap();

        assert_eq!(head_content(dir.path(), Path::new("plain.txt")), None);
    }

    #[test]
    fn head_content_is_none_in_repo_without_commits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("f.txt"), "x\n").unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["init", "--quiet"])
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(cmd.status().unwrap().success(), "git init failed");

        assert_eq!(head_content(root, Path::new("f.txt")), None);
    }

    #[test]
    fn head_content_is_none_for_non_utf8_blob() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("blob.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["init", "--quiet"])
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(cmd.status().unwrap().success());
        let mut add = Command::new("git");
        add.args(["add", "."])
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(add.status().unwrap().success());
        let mut commit = Command::new("git");
        commit
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "initial",
            ])
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(commit.status().unwrap().success(), "git commit failed");

        assert_eq!(head_content(root, Path::new("blob.bin")), None);
    }

    #[test]
    fn head_content_resolves_from_subdirectory_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo_root = dir.path().join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["init", "--quiet"])
            .current_dir(&repo_root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(cmd.status().unwrap().success(), "git init failed");
        fs::create_dir_all(repo_root.join("crate/src")).unwrap();
        fs::write(repo_root.join("crate/src/lib.rs"), "committed text\n").unwrap();
        let mut add = Command::new("git");
        add.args(["add", "."])
            .current_dir(&repo_root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(add.status().unwrap().success());
        let mut commit = Command::new("git");
        commit
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "initial",
            ])
            .current_dir(&repo_root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(commit.status().unwrap().success(), "git commit failed");

        let project_root = repo_root.join("crate");
        fs::write(project_root.join("src/lib.rs"), "bash rewrote it\n").unwrap();

        assert_eq!(
            head_content(&project_root, Path::new("src/lib.rs")),
            Some("committed text\n".to_string()),
            "the project root may be a subdirectory of the repo; \
             the lookup resolves the relative path from it"
        );
    }
}
