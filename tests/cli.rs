use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_agent-sync")
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .arg("--home")
        .arg(home)
        .args(args)
        .output()
        .expect("agent-sync should run")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent")).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn imports_applies_and_reaches_an_idempotent_state() {
    let fixture = TempDir::new().unwrap();
    let home = fixture.path().join("home");
    let profile = fixture.path().join("profile");
    write_file(
        &home.join(".claude/skills/reviewer/SKILL.md"),
        "---\nname: reviewer\ndescription: Review code carefully.\n---\n\nReview the diff and report concrete issues.\n",
    );
    write_file(
        &home.join(".claude/skills/reviewer/references/checklist.md"),
        "- correctness\n- tests\n",
    );
    write_file(
        &home.join(".claude/skills/reviewer/.env"),
        "SECRET=do-not-copy\n",
    );
    write_file(
        &home.join(".claude/history/session.sqlite"),
        "private history\n",
    );
    write_file(
        &home.join(".claude/CLAUDE.md"),
        "# Project preferences\n\nUse small, tested changes.\n",
    );

    let init = run(&home, &["init", "--profile", profile.to_str().unwrap()]);
    assert_success(&init);

    let scan = run(&home, &["scan", "--agent", "claude", "--scope", "global"]);
    assert_success(&scan);
    let scan_text = text(&scan);
    assert!(scan_text.contains("reviewer"));
    assert!(!scan_text.contains(".env"));
    assert!(!scan_text.contains("session.sqlite"));

    let import = run(
        &home,
        &[
            "import",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
        ],
    );
    assert_success(&import);
    assert!(profile.join("global/skills/reviewer/SKILL.md").exists());
    assert!(!profile.join("global/skills/reviewer/.env").exists());
    assert!(!profile.join("global/history/session.sqlite").exists());

    let plan = run(
        &home,
        &[
            "plan",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
        ],
    );
    assert_success(&plan);
    assert!(text(&plan).contains("update"));

    let apply = run(
        &home,
        &[
            "apply",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
            "--yes",
        ],
    );
    assert_success(&apply);
    assert!(profile.join("state/manifest.json").exists());

    let status = run(
        &home,
        &[
            "status",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
        ],
    );
    assert_success(&status);
    assert!(text(&status).contains("0 create"));
    assert!(text(&status).contains("0 conflict"));
}

#[test]
fn project_target_edits_are_reported_as_conflicts() {
    let fixture = TempDir::new().unwrap();
    let home = fixture.path().join("home");
    let project = fixture.path().join("project");
    let profile = fixture.path().join("profile");
    fs::create_dir_all(project.join(".git")).unwrap();
    write_file(
        &project.join("CLAUDE.md"),
        "# Team rules\n\nRun tests before committing.\n",
    );

    assert_success(&run(
        &home,
        &["init", "--profile", profile.to_str().unwrap()],
    ));
    let import = run(
        &home,
        &[
            "import",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "project",
            "--project",
            project.to_str().unwrap(),
        ],
    );
    assert_success(&import);

    let apply = run(
        &home,
        &[
            "apply",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "project",
            "--project",
            project.to_str().unwrap(),
            "--yes",
        ],
    );
    assert_success(&apply);

    write_file(
        &project.join("CLAUDE.md"),
        "# Team rules\n\nDo something different without updating the profile.\n",
    );
    let status = run(
        &home,
        &[
            "status",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "project",
            "--project",
            project.to_str().unwrap(),
        ],
    );
    assert_eq!(status.status.code(), Some(2));
    assert!(text(&status).contains("conflict"));
}

#[test]
fn malformed_skills_fail_scan_without_writing() {
    let fixture = TempDir::new().unwrap();
    let home = fixture.path().join("home");
    let profile = fixture.path().join("profile");
    write_file(
        &home.join(".gemini/skills/broken/SKILL.md"),
        "This skill has no frontmatter.\n",
    );
    assert_success(&run(
        &home,
        &["init", "--profile", profile.to_str().unwrap()],
    ));
    let scan = run(&home, &["scan", "--agent", "gemini", "--scope", "global"]);
    assert!(!scan.status.success());
    assert!(text(&scan).contains("frontmatter"));
    assert!(!profile.join("global/skills/broken").exists());
}

#[test]
fn adapters_command_lists_the_eight_builtins() {
    let fixture = TempDir::new().unwrap();
    let output = run(fixture.path(), &["adapters"]);
    assert_success(&output);
    let output = text(&output);
    for adapter in [
        "claude", "codex", "gemini", "cline", "cursor", "copilot", "opencode", "aider",
    ] {
        assert!(output.contains(adapter), "missing adapter {adapter}");
    }
}

#[test]
fn manifest_keys_remain_portable_between_machine_home_directories() {
    let fixture = TempDir::new().unwrap();
    let home_one = fixture.path().join("home-one");
    let home_two = fixture.path().join("home-two");
    let profile = fixture.path().join("profile");
    write_file(
        &home_one.join(".claude/skills/portable/SKILL.md"),
        "---\nname: portable\ndescription: A portable skill.\n---\n\nDo the portable thing.\n",
    );

    assert_success(&run(
        &home_one,
        &["init", "--profile", profile.to_str().unwrap()],
    ));
    assert_success(&run(
        &home_one,
        &[
            "import",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
        ],
    ));
    assert_success(&run(
        &home_one,
        &[
            "apply",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
            "--yes",
        ],
    ));

    let status_on_second_machine = run(
        &home_two,
        &[
            "status",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
        ],
    );
    assert_success(&status_on_second_machine);
    assert!(text(&status_on_second_machine).contains("create"));

    assert_success(&run(
        &home_two,
        &[
            "apply",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
            "--yes",
        ],
    ));
    let manifest = fs::read_to_string(profile.join("state/manifest.json")).unwrap();
    assert_eq!(manifest.matches("\"agent\"").count(), 1);
    let final_status = run(
        &home_two,
        &[
            "status",
            "--profile",
            profile.to_str().unwrap(),
            "--agent",
            "claude",
            "--scope",
            "global",
        ],
    );
    assert_success(&final_status);
    assert!(text(&final_status).contains("0 delete-pending"));
}
