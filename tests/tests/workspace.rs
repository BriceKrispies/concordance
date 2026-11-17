//! A workspace whose spec and source roots are separate git repositories under a plain
//! parent directory.

use concordance_core::config::{Config, ConfigFile, Overrides, RepoFile};
use concordance_engine::citation::summarize;
use concordance_tests::*;
use std::path::PathBuf;

/// `repo` is a plain parent directory; the spec root and two TypeScript packages are
/// separate git repositories, each with its own `.gitignore`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_of_separate_git_repositories() {
    let root = temp_dir("workspace");
    let ws = root.join("ws");
    let specs = ws.join("specs");
    write(
        &specs.join("system.md"),
        "# System\n\nThe entry point is `start()` [PB:src/b.ts:1].\n",
    );
    write(&specs.join("drafts/wip.md"), "# Draft\n");
    write(&specs.join(".gitignore"), "drafts/\n");
    let a = ws.join("pkg-a");
    write(&a.join("package.json"), r#"{ "name": "@ws/alpha" }"#);
    write(
        &a.join("src/index.ts"),
        "export function start(): void {}\n",
    );
    write(
        &a.join("dist/index.ts"),
        "export function built(): void {}\n",
    );
    write(&a.join(".gitignore"), "dist/\n");
    let b = ws.join("pkg-b");
    write(&b.join("package.json"), r#"{ "name": "beta" }"#);
    write(&b.join("src/b.ts"), "export const b = 1;\n");
    for d in [&specs, &a, &b] {
        git(d, &["init", "-q", "-b", "main"]);
        git(d, &["add", "-A"]);
        git(d, &["commit", "-q", "-m", "init"]);
    }
    let state = root.join("state");
    let mut file = ConfigFile::default();
    file.repos.insert(
        "PB".into(),
        RepoFile {
            path: PathBuf::from("pkg-b"),
            cited: "HEAD".into(),
            ..Default::default()
        },
    );
    let cfg = Config::resolve(
        file,
        Some(&ws),
        Overrides {
            repo: Some(ws.clone()),
            spec_roots: vec![PathBuf::from("specs")],
            src_roots: vec![PathBuf::from("pkg-a"), PathBuf::from("pkg-b")],
            state_dir: Some(state.clone()),
            ..Default::default()
        },
        &ws,
        &state,
    )
    .unwrap();
    let repos = cfg.repos.clone();
    let r = start(cfg, None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    let files: Vec<&str> = m.files.values().map(|f| f.path.as_str()).collect();
    assert!(files.contains(&"pkg-a/src/index.ts"), "{files:?}");
    assert!(files.contains(&"pkg-b/src/b.ts"), "{files:?}");
    assert!(!files.contains(&"pkg-a/dist/index.ts"), "{files:?}");
    let docs: Vec<&str> = m.documents.values().map(|d| d.path.as_str()).collect();
    assert_eq!(docs, vec!["specs/system.md"]);
    let pkgs: Vec<&str> = m.workspace.crates.iter().map(|c| c.name.as_str()).collect();
    assert!(
        pkgs.contains(&"alpha") && pkgs.contains(&"beta"),
        "{pkgs:?}"
    );
    assert!(m.symbols.values().any(|s| s.name == "start"));
    let s = summarize(&m, &repos).unwrap();
    assert_eq!(s.aliases[0].resolved, 1, "{s:?}");

    // The watcher applies each sub-repository's .gitignore too.
    write(
        &a.join("dist/later.ts"),
        "export function later(): void {}\n",
    );
    write(&a.join("src/later.ts"), "export function kept(): void {}\n");
    r.engine
        .notify_changed(vec![a.join("dist/later.ts"), a.join("src/later.ts")])
        .await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    let files: Vec<&str> = m.files.values().map(|f| f.path.as_str()).collect();
    assert!(files.contains(&"pkg-a/src/later.ts"), "{files:?}");
    assert!(!files.contains(&"pkg-a/dist/later.ts"), "{files:?}");
    r.stop().await;
}
