use crate::skills::state::{LoadedReference, ScriptResult};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use tokio::process::Command;

const MAX_REFERENCE_BYTES: usize = 256 * 1024;
const MAX_STDOUT_BYTES: usize = 32 * 1024;
const MAX_STDERR_BYTES: usize = 32 * 1024;
const SCRIPT_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum SkillMaterializeError {
    #[error("invalid relative path: {0}")]
    InvalidPath(String),
    #[error("path is outside skill root")]
    PathEscapesRoot,
    #[error("unsupported path (expected under {0})")]
    UnsupportedPath(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("script runtime not supported for: {0}")]
    UnsupportedRuntime(String),
    #[error("script timed out after {0}s")]
    Timeout(u64),
}

pub(crate) fn load_reference_material(
    skill_id: &str,
    skill_root: &Path,
    relative_path: &str,
) -> Result<LoadedReference, SkillMaterializeError> {
    let rel = normalize_relative_path(relative_path)?;
    require_prefix(&rel, "references")?;
    let full = resolve_under_root(skill_root, &rel)?;

    let bytes = std::fs::read(&full).map_err(|e| SkillMaterializeError::Io(e.to_string()))?;
    let original_len = bytes.len() as u64;

    let (stored_bytes, truncated) = if bytes.len() > MAX_REFERENCE_BYTES {
        (bytes[..MAX_REFERENCE_BYTES].to_vec(), true)
    } else {
        (bytes, false)
    };

    let content = String::from_utf8(stored_bytes.clone())
        .map_err(|e| SkillMaterializeError::Io(format!("invalid utf-8: {e}")))?;
    let sha256 = sha256_hex(&stored_bytes);

    Ok(LoadedReference {
        skill: skill_id.to_string(),
        path: rel.to_string_lossy().to_string(),
        sha256,
        truncated,
        content,
        bytes: original_len,
    })
}

pub(crate) async fn run_script_material(
    skill_id: &str,
    skill_root: &Path,
    script_path: &str,
    args: &[String],
) -> Result<ScriptResult, SkillMaterializeError> {
    let rel = normalize_relative_path(script_path)?;
    require_prefix(&rel, "scripts")?;
    let full = resolve_under_root(skill_root, &rel)?;

    let (program, mut cmd_args) = runtime_for_script(&full)?;
    cmd_args.push(full.to_string_lossy().to_string());
    cmd_args.extend(args.iter().cloned());

    let mut cmd = Command::new(program);
    cmd.args(cmd_args);
    cmd.current_dir(skill_root);
    cmd.kill_on_drop(true);

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(SCRIPT_TIMEOUT_SECS),
        cmd.output(),
    )
    .await
    .map_err(|_| SkillMaterializeError::Timeout(SCRIPT_TIMEOUT_SECS))?
    .map_err(|e| SkillMaterializeError::Io(e.to_string()))?;

    let exit_code = out.status.code().unwrap_or(-1);

    let (stdout, truncated_stdout) = truncate_bytes(&out.stdout, MAX_STDOUT_BYTES);
    let (stderr, truncated_stderr) = truncate_bytes(&out.stderr, MAX_STDERR_BYTES);

    let mut hasher = Sha256::new();
    hasher.update(&out.stdout);
    hasher.update(&out.stderr);
    let sha256 = hex_encode(&hasher.finalize());

    Ok(ScriptResult {
        skill: skill_id.to_string(),
        script: rel.to_string_lossy().to_string(),
        sha256,
        truncated_stdout,
        truncated_stderr,
        exit_code,
        stdout,
        stderr,
    })
}

fn runtime_for_script(script: &Path) -> Result<(String, Vec<String>), SkillMaterializeError> {
    let ext = script.extension().and_then(|s| s.to_str()).unwrap_or("");
    match ext {
        "sh" => Ok(("bash".to_string(), vec![])),
        "py" => Ok(("python3".to_string(), vec![])),
        "js" => Ok(("node".to_string(), vec![])),
        _ => Err(SkillMaterializeError::UnsupportedRuntime(
            script.to_string_lossy().to_string(),
        )),
    }
}

fn truncate_bytes(bytes: &[u8], max: usize) -> (String, bool) {
    if bytes.len() <= max {
        return (String::from_utf8_lossy(bytes).to_string(), false);
    }
    let truncated = &bytes[..max];
    (String::from_utf8_lossy(truncated).to_string(), true)
}

fn normalize_relative_path(p: &str) -> Result<PathBuf, SkillMaterializeError> {
    if p.trim().is_empty() {
        return Err(SkillMaterializeError::InvalidPath("empty".to_string()));
    }
    let raw = Path::new(p);
    if raw.is_absolute() {
        return Err(SkillMaterializeError::InvalidPath("absolute".to_string()));
    }
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => {
                return Err(SkillMaterializeError::InvalidPath("absolute".to_string()));
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(SkillMaterializeError::InvalidPath("parent_dir".to_string()));
            }
            Component::Normal(seg) => out.push(seg),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(SkillMaterializeError::InvalidPath("empty".to_string()));
    }
    Ok(out)
}

fn require_prefix(rel: &Path, required_first_component: &str) -> Result<(), SkillMaterializeError> {
    let first = rel
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("");
    if first != required_first_component {
        return Err(SkillMaterializeError::UnsupportedPath(
            required_first_component.to_string(),
        ));
    }
    Ok(())
}

fn resolve_under_root(root: &Path, rel: &Path) -> Result<PathBuf, SkillMaterializeError> {
    let root = std::fs::canonicalize(root).map_err(|e| SkillMaterializeError::Io(e.to_string()))?;
    let joined = root.join(rel);
    let canon =
        std::fs::canonicalize(&joined).map_err(|e| SkillMaterializeError::Io(e.to_string()))?;
    if !canon.starts_with(&root) {
        return Err(SkillMaterializeError::PathEscapesRoot);
    }
    Ok(canon)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn load_reference_requires_references_prefix() {
        let td = TempDir::new().unwrap();
        fs::create_dir_all(td.path().join("docx").join("references")).unwrap();
        fs::write(
            td.path().join("docx").join("references").join("a.md"),
            "hello",
        )
        .unwrap();

        let err = load_reference_material("docx", &td.path().join("docx"), "a.md")
            .unwrap_err()
            .to_string();
        assert!(err.contains("expected under references"));
    }

    #[test]
    fn load_reference_allows_nested_paths() {
        let td = TempDir::new().unwrap();
        fs::create_dir_all(td.path().join("docx").join("references").join("nested")).unwrap();
        fs::write(
            td.path()
                .join("docx")
                .join("references")
                .join("nested")
                .join("a.md"),
            "hello",
        )
        .unwrap();

        let mat =
            load_reference_material("docx", &td.path().join("docx"), "references/nested/a.md")
                .expect("nested references are allowed");
        assert_eq!(mat.path, "references/nested/a.md");
        assert_eq!(mat.content.trim(), "hello");
    }

    #[tokio::test]
    async fn run_script_rejects_path_escape() {
        let td = TempDir::new().unwrap();
        fs::create_dir_all(td.path().join("s").join("scripts")).unwrap();
        let err = run_script_material("s", &td.path().join("s"), "../x.sh", &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid relative path"));
    }
}
