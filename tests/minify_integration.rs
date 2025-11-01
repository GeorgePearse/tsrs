use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn copy_dir_filtered(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "__pycache__" {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir_filtered(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, dst_path)?;
        }
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct MinifyPlan {
    module: String,
    functions: Vec<FunctionPlan>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct FunctionPlan {
    qualified_name: String,
    locals: Vec<String>,
    renames: Vec<RenameEntry>,
    #[serde(default)]
    has_match_statement: Option<bool>,
    #[serde(default)]
    has_comprehension: Option<bool>,
    #[serde(default)]
    has_nested_functions: Option<bool>,
    #[serde(default)]
    has_imports: Option<bool>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct RenameEntry {
    original: String,
    renamed: String,
}

#[derive(Debug, Deserialize)]
struct PlanBundle {
    version: u32,
    files: Vec<PlanFile>,
}

#[derive(Debug, Deserialize)]
struct PlanFile {
    path: String,
    plan: MinifyPlan,
}

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test_packages/test_minify")
        .join(relative)
}

const PATTERNS_SOURCE: &str = include_str!("../test_packages/test_minify/src/patterns.py");

fn extract_pattern_match_source() -> &'static str {
    PATTERNS_SOURCE
        .split("PATTERN_MATCH_SOURCE = \"\"\"")
        .nth(1)
        .and_then(|rest| rest.split("\"\"\"").next())
        .expect("pattern source constant missing")
}

fn materialize_pattern_fixture(dst_dir: &Path) -> Result<()> {
    let pattern_path = dst_dir.join("patterns.py");
    let content = format!(
        "\"\"\"Structural pattern matching fixture for minify tests.\"\"\"\n\n{}",
        extract_pattern_match_source()
    );
    fs::write(pattern_path, content)?;
    Ok(())
}

fn touch_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

#[test]
fn minify_plan_emits_expected_json() -> Result<()> {
    let temp = TempDir::new()?;
    let src = fixture_path("src/simple_module.py");
    let dst = temp.path().join("simple_module.py");
    fs::copy(&src, &dst)?;

    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-plan")
        .arg(&dst)
        .output()
        .context("failed to execute tsrs-cli minify-plan")?;

    anyhow::ensure!(
        output.status.success(),
        "minify-plan exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout)?;
    let plan: MinifyPlan = serde_json::from_str(&stdout)?;

    assert_eq!(plan.module, "simple_module");

    let function = plan
        .functions
        .iter()
        .find(|func| func.locals.iter().any(|name| name == "message"))
        .context("expected plan to include greet locals")?;

    assert!(
        function
            .renames
            .iter()
            .any(|rename| rename.original == "message")
    );

    Ok(())
}

#[test]
fn minify_in_place_rewrites_locals() -> Result<()> {
    let temp = TempDir::new()?;
    let src = fixture_path("src/simple_module.py");
    let dst = temp.path().join("simple_module.py");
    fs::copy(&src, &dst)?;

    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify")
        .arg(&dst)
        .arg("--in-place")
        .output()
        .context("failed to execute tsrs-cli minify")?;

    anyhow::ensure!(
        output.status.success(),
        "minify exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let rewritten = fs::read_to_string(&dst)?;
    assert!(!rewritten.contains("message"));
    assert!(!rewritten.contains("suffix"));
    assert!(rewritten.contains("Hello"));

    Ok(())
}

#[test]
fn minify_plan_dir_outputs_bundle_with_expected_files() -> Result<()> {
    let temp = TempDir::new()?;
    let src_dir = fixture_path("src");
    let dst_dir = temp.path().join("src");
    copy_dir_filtered(&src_dir, &dst_dir)?;
    materialize_pattern_fixture(&dst_dir)?;

    let plan_path = temp.path().join("plan.json");

    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-plan-dir")
        .arg(&dst_dir)
        .arg("--out")
        .arg(&plan_path)
        .output()
        .context("failed to execute tsrs-cli minify-plan-dir")?;

    anyhow::ensure!(
        output.status.success(),
        "minify-plan-dir exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let bundle_contents = fs::read_to_string(&plan_path)?;
    let bundle: PlanBundle = serde_json::from_str(&bundle_contents)?;

    assert_eq!(bundle.version, 1);

    let file_set: HashSet<&str> = bundle.files.iter().map(|file| file.path.as_str()).collect();
    assert!(file_set.contains("simple_module.py"));
    assert!(file_set.contains("nested/calculator.py"));
    assert!(file_set.contains("patterns.py"));
    assert!(file_set.contains("comprehension_module.py"));
    assert!(file_set.contains("class_module.py"));
    assert!(file_set.contains("class_methods.py"));

    for file in &bundle.files {
        assert!(
            file
                .plan
                .functions
                .iter()
                .any(|func| !func.renames.is_empty()),
            "expected at least one rename in {}",
            file.path
        );
    }

    let patterns_plan = bundle
        .files
        .iter()
        .find(|file| file.path == "patterns.py")
        .context("patterns.py plan missing")?;
    assert!(patterns_plan
        .plan
        .functions
        .iter()
        .any(|func| func.has_match_statement.unwrap_or(false)));

    let comprehension_plan = bundle
        .files
        .iter()
        .find(|file| file.path == "comprehension_module.py")
        .context("comprehension_module.py plan missing")?;
    assert!(comprehension_plan
        .plan
        .functions
        .iter()
        .any(|func| func.has_comprehension.unwrap_or(false)));

    let class_plan = bundle
        .files
        .iter()
        .find(|file| file.path == "class_methods.py")
        .context("class_methods.py plan missing")?;
    assert!(class_plan
        .plan
        .functions
        .iter()
        .any(|func| func.has_nested_functions.unwrap_or(false)));
    assert!(class_plan
        .plan
        .functions
        .iter()
        .any(|func| func.has_imports.unwrap_or(false)));

    Ok(())
}

#[test]
fn minify_plan_handles_class_module_reserved_names() -> Result<()> {
    let temp = TempDir::new()?;
    let src = fixture_path("src/class_module.py");
    let dst = temp.path().join("class_module.py");
    fs::copy(&src, &dst)?;

    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-plan")
        .arg(&dst)
        .output()
        .context("failed to execute tsrs-cli minify-plan for class_module")?;

    anyhow::ensure!(
        output.status.success(),
        "minify-plan exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout)?;
    let plan: MinifyPlan = serde_json::from_str(&stdout)?;

    assert_eq!(plan.module, "class_module");

    assert!(plan
        .functions
        .iter()
        .any(|func| func.qualified_name == "Greeter.greet"));

    let reserved_untouched = plan.functions.iter().all(|func| {
        func.renames
            .iter()
            .all(|rename| rename.original != "self" && rename.original != "cls")
    });
    assert!(reserved_untouched, "expected self/cls to remain untouched");

    let greet_plan = plan
        .functions
        .iter()
        .find(|func| func.qualified_name == "Greeter.greet")
        .context("missing plan for Greeter.greet")?;
    assert!(greet_plan.has_nested_functions.unwrap_or(false));
    assert!(greet_plan.has_imports.unwrap_or(false));

    let describe_plan = plan
        .functions
        .iter()
        .find(|func| func.qualified_name == "Greeter.describe")
        .context("missing plan for Greeter.describe")?;
    assert!(describe_plan.has_nested_functions.unwrap_or(false));

    Ok(())
}

#[test]
fn minify_plan_dir_hidden_files_behavior() -> Result<()> {
    let temp = TempDir::new()?;
    let src_dir = fixture_path("src");
    let dst_dir = temp.path().join("src");
    copy_dir_filtered(&src_dir, &dst_dir)?;
    let hidden = dst_dir.join(".hidden_module.py");
    touch_file(&hidden, "def demo():\n    value = 1\n    return value\n")?;

    let plan_path = temp.path().join("plan.json");
    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-plan-dir")
        .arg(&dst_dir)
        .arg("--out")
        .arg(&plan_path)
        .output()
        .context("failed to execute minify-plan-dir without include-hidden")?;
    anyhow::ensure!(
        output.status.success(),
        "minify-plan-dir exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle: PlanBundle = serde_json::from_slice(&fs::read(&plan_path)?)?;
    assert!(!bundle.files.iter().any(|file| file.path.contains(".hidden_module")));

    let hidden_plan_path = temp.path().join("plan-hidden.json");
    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-plan-dir")
        .arg(&dst_dir)
        .arg("--out")
        .arg(&hidden_plan_path)
        .arg("--include-hidden")
        .output()
        .context("failed to execute minify-plan-dir with include-hidden")?;
    anyhow::ensure!(
        output.status.success(),
        "minify-plan-dir --include-hidden exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle_hidden: PlanBundle = serde_json::from_slice(&fs::read(&hidden_plan_path)?)?;
    assert!(bundle_hidden
        .files
        .iter()
        .any(|file| file.path.contains(".hidden_module")));

    Ok(())
}

#[test]
fn minify_plan_dir_respects_gitignore() -> Result<()> {
    let temp = TempDir::new()?;
    let src_dir = fixture_path("src");
    let dst_dir = temp.path().join("src");
    copy_dir_filtered(&src_dir, &dst_dir)?;
    let ignored_path = dst_dir.join("ignored_module.py");
    touch_file(&ignored_path, "def ignored():\n    value = 2\n    return value\n")?;
    touch_file(&dst_dir.join(".gitignore"), "ignored_module.py\n")?;

    let plan_path = temp.path().join("plan.json");
    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-plan-dir")
        .arg(&dst_dir)
        .arg("--out")
        .arg(&plan_path)
        .output()
        .context("failed to execute minify-plan-dir default")?;
    anyhow::ensure!(
        output.status.success(),
        "minify-plan-dir exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle: PlanBundle = serde_json::from_slice(&fs::read(&plan_path)?)?;
    assert!(bundle
        .files
        .iter()
        .any(|file| file.path == "ignored_module.py"));

    let respect_path = temp.path().join("plan-respect.json");
    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-plan-dir")
        .arg(&dst_dir)
        .arg("--out")
        .arg(&respect_path)
        .arg("--respect-gitignore")
        .output()
        .context("failed to execute minify-plan-dir with respect-gitignore")?;
    anyhow::ensure!(
        output.status.success(),
        "minify-plan-dir --respect-gitignore exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle_respect: PlanBundle = serde_json::from_slice(&fs::read(&respect_path)?)?;
    assert!(!bundle_respect
        .files
        .iter()
        .any(|file| file.path == "ignored_module.py"));

    Ok(())
}

#[test]
fn minify_dir_in_place_rewrites_files() -> Result<()> {
    let temp = TempDir::new()?;
    let src = fixture_path("src/simple_module.py");
    let dst_dir = temp.path().join("workspace");
    fs::create_dir_all(&dst_dir)?;
    fs::copy(&src, dst_dir.join("simple_module.py"))?;

    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify-dir")
        .arg(&dst_dir)
        .arg("--in-place")
        .output()
        .context("failed to execute minify-dir --in-place")?;
    anyhow::ensure!(
        output.status.success(),
        "minify-dir exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let rewritten = fs::read_to_string(dst_dir.join("simple_module.py"))?;
    assert!(!rewritten.contains("message"));
    assert!(!rewritten.contains("suffix"));

    Ok(())
}

#[test]
fn minify_stdout_emits_rewritten_code_without_touching_file() -> Result<()> {
    let temp = TempDir::new()?;
    let src = fixture_path("src/simple_module.py");
    let dst = temp.path().join("simple_module.py");
    fs::copy(&src, &dst)?;
    let original = fs::read_to_string(&dst)?;

    let output = assert_cmd::cargo::cargo_bin_cmd!("tsrs-cli")
        .arg("minify")
        .arg(&dst)
        .arg("--stdout")
        .output()
        .context("failed to execute minify --stdout")?;

    anyhow::ensure!(
        output.status.success(),
        "minify --stdout exited with {}. stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Hello"));
    assert!(!stdout.contains("message"));

    let after = fs::read_to_string(&dst)?;
    assert_eq!(after, original, "source file should remain unchanged");

    Ok(())
}
