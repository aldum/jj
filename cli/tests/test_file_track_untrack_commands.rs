// Copyright 2022 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::common::TestEnvironment;

#[test]
fn test_track_untrack() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "initial").unwrap();
    std::fs::write(repo_path.join("file1.bak"), "initial").unwrap();
    std::fs::write(repo_path.join("file2.bak"), "initial").unwrap();
    let target_dir = repo_path.join("target");
    std::fs::create_dir(&target_dir).unwrap();
    std::fs::write(target_dir.join("file2"), "initial").unwrap();
    std::fs::write(target_dir.join("file3"), "initial").unwrap();

    // Run a command so all the files get tracked, then add "*.bak" to the ignore
    // patterns
    test_env.run_jj_in(&repo_path, ["st"]).success();
    std::fs::write(repo_path.join(".gitignore"), "*.bak\n").unwrap();
    let files_before = test_env.run_jj_in(&repo_path, ["file", "list"]).success();

    // Errors out when not run at the head operation
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack", "file1", "--at-op", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command must be able to update the working copy.
    Hint: Don't use --at-op.
    [EOF]
    [exit status: 1]
    ");
    // Errors out when no path is specified
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    error: the following required arguments were not provided:
      <FILESETS>...

    Usage: jj file untrack <FILESETS>...

    For more information, try '--help'.
    [EOF]
    [exit status: 2]
    ");
    // Errors out when a specified file is not ignored
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack", "file1", "file1.bak"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: 'file1' is not ignored.
    Hint: Files that are not ignored will be added back by the next command.
    Make sure they're ignored, then try again.
    [EOF]
    [exit status: 1]
    ");
    let files_after = test_env.run_jj_in(&repo_path, ["file", "list"]).success();
    // There should be no changes to the state when there was an error
    assert_eq!(files_after, files_before);

    // Can untrack a single file
    assert!(files_before.stdout.raw().contains("file1.bak\n"));
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack", "file1.bak"]);
    insta::assert_snapshot!(output, @r"");
    let files_after = test_env.run_jj_in(&repo_path, ["file", "list"]).success();
    // The file is no longer tracked
    assert!(!files_after.stdout.raw().contains("file1.bak"));
    // Other files that match the ignore pattern are not untracked
    assert!(files_after.stdout.raw().contains("file2.bak"));
    // The files still exist on disk
    assert!(repo_path.join("file1.bak").exists());
    assert!(repo_path.join("file2.bak").exists());

    // Errors out when multiple specified files are not ignored
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack", "target"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Error: 'target/file2' and 1 other files are not ignored.
    Hint: Files that are not ignored will be added back by the next command.
    Make sure they're ignored, then try again.
    [EOF]
    [exit status: 1]
    ");

    // Can untrack after adding to ignore patterns
    std::fs::write(repo_path.join(".gitignore"), ".bak\ntarget/\n").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack", "target"]);
    insta::assert_snapshot!(output, @"");
    let files_after = test_env.run_jj_in(&repo_path, ["file", "list"]).success();
    assert!(!files_after.stdout.raw().contains("target"));
}

#[test]
fn test_track_untrack_sparse() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "contents").unwrap();
    std::fs::write(repo_path.join("file2"), "contents").unwrap();

    // When untracking a file that's not included in the sparse working copy, it
    // doesn't need to be ignored (because it won't be automatically added
    // back).
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    file2
    [EOF]
    ");
    test_env
        .run_jj_in(&repo_path, ["sparse", "set", "--clear", "--add", "file1"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack", "file2"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
    // Trying to manually track a file that's not included in the sparse working has
    // no effect. TODO: At least a warning would be useful
    let output = test_env.run_jj_in(&repo_path, ["file", "track", "file2"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
}

#[test]
fn test_auto_track() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"snapshot.auto-track = 'glob:*.rs'"#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1.rs"), "initial").unwrap();
    std::fs::write(repo_path.join("file2.md"), "initial").unwrap();
    std::fs::write(repo_path.join("file3.md"), "initial").unwrap();

    // Only configured paths get auto-tracked
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1.rs
    [EOF]
    ");

    // Can manually track paths
    let output = test_env.run_jj_in(&repo_path, ["file", "track", "file3.md"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1.rs
    file3.md
    [EOF]
    ");

    // Can manually untrack paths
    let output = test_env.run_jj_in(&repo_path, ["file", "untrack", "file3.md"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1.rs
    [EOF]
    ");

    // CWD-relative paths in `snapshot.auto-track` are evaluated from the repo root
    let subdir = repo_path.join("sub");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(subdir.join("file1.rs"), "initial").unwrap();
    let output = test_env.run_jj_in(&subdir, ["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ../file1.rs
    [EOF]
    ");

    // But `jj file track` wants CWD-relative paths
    let output = test_env.run_jj_in(&subdir, ["file", "track", "file1.rs"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&subdir, ["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ../file1.rs
    file1.rs
    [EOF]
    ");
}

#[test]
fn test_track_ignored() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"snapshot.auto-track = 'none()'"#);
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join(".gitignore"), "*.bak\n").unwrap();
    std::fs::write(repo_path.join("file1"), "initial").unwrap();
    std::fs::write(repo_path.join("file1.bak"), "initial").unwrap();

    // Track an unignored path
    let output = test_env.run_jj_in(&repo_path, ["file", "track", "file1"]);
    insta::assert_snapshot!(output, @"");
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
    // Track an ignored path
    let output = test_env.run_jj_in(&repo_path, ["file", "track", "file1.bak"]);
    insta::assert_snapshot!(output, @"");
    // TODO: We should teach `jj file track` to track ignored paths (possibly
    // requiring a flag)
    let output = test_env.run_jj_in(&repo_path, ["file", "list"]);
    insta::assert_snapshot!(output, @r"
    file1
    [EOF]
    ");
}
