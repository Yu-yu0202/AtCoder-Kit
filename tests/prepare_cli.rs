use std::fs;
use std::process::Command;

fn workspace(template_change: impl FnOnce(&mut serde_json::Value)) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("abc999");
    let problem = root.join("a");
    fs::create_dir_all(&problem).unwrap();
    fs::write(
        root.join("contest.json"),
        include_bytes!("fixtures/json/contest.json"),
    )
    .unwrap();
    let mut template: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/json/template_legacy.json")).unwrap();
    template_change(&mut template);
    fs::write(
        problem.join("template.json"),
        serde_json::to_vec(&template).unwrap(),
    )
    .unwrap();
    fs::write(problem.join("main.py"), "print(3)\n").unwrap();
    temp
}

fn prepare(temp: &tempfile::TempDir, no_test: bool) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ackit"));
    command.current_dir(temp.path().join("abc999/a"));
    command.arg("prepare");
    if no_test {
        command.arg("--no-test");
    }
    command.output().unwrap()
}

#[test]
fn prepare_prints_only_normalized_source() {
    let temp = workspace(|_| {});
    let output = prepare(&temp, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"print(3)\r\n");
}

#[test]
fn prepare_failure_does_not_print_source() {
    let temp = workspace(|template| {
        template["exec_command"] = serde_json::json!(["ackit-no-such-program"]);
    });
    let output = prepare(&temp, false);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn no_test_still_runs_pre_submit() {
    let temp = workspace(|template| {
        template["pre_submit"] = serde_json::json!(["ackit-no-such-program"]);
    });
    let output = prepare(&temp, true);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
